//! Shared helpers for the tests of the identity worker.
//!
//! The rules of the identity are the rules of a database: a magic link that
//! works once, a refresh family that dies with a stolen token. The tests
//! that prove them run against a real Postgres.
//!
//! The tests that only prove a rule of the domain run without a database:
//! the clock is injected, so the expiry and the rotation are deterministic.

#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use futures_util::future::BoxFuture;
use sqlx::postgres::PgPool;
use suertudo_db::pool;
use suertudo_db::{TenantContext, with_tenant};
use suertudo_identity::adapters::{
    Hs256TokenSigner, PostgresConsentRepository, PostgresDeviceRepository, PostgresTokenRepository,
    PostgresUserRepository, RandomTokenSource, Sha256Digest,
};
use suertudo_identity::domain::service::IdentityService;
use suertudo_identity::domain::types::StoredToken;
use suertudo_identity::handlers::Deps;
use suertudo_identity::middleware::SessionResolver;
use suertudo_identity::ports::{
    Clock, SmsError, SmsMessage, SmsSender, StorageError, TokenRepository,
};
use tokio::sync::OnceCell;

/// The secret of the access tokens in the tests.
pub const ACCESS_SECRET: &str = "test-access-secret-0123456789abcdef";
/// The secret of the refresh tokens in the tests.
pub const REFRESH_SECRET: &str = "test-refresh-secret-0123456789abcdef";

/// The setup of the test database, done once per test binary.
static SETUP: OnceCell<()> = OnceCell::const_new();

/// The role of the application, prepared once per test binary.
static APP_ROLE: OnceCell<()> = OnceCell::const_new();

/// The connection string of the test database.
pub fn database_url() -> String {
    std::env::var("DATABASE_URL_TEST")
        .unwrap_or_else(|_| "postgres://suertudo:suertudo@127.0.0.1:5432/suertudo_test".to_string())
}

/// A pool with the migrations applied.
pub async fn test_pool() -> Result<PgPool, Box<dyn std::error::Error>> {
    let pool = pool::connect(&database_url(), 5).await?;
    SETUP
        .get_or_try_init(|| async {
            pool::migrate(&pool).await?;
            Ok::<_, Box<dyn std::error::Error>>(())
        })
        .await?;
    Ok(pool)
}

/// A clock that only moves when the test moves it.
#[derive(Debug)]
pub struct TestClock {
    now: AtomicI64,
}

impl TestClock {
    /// A clock at a fixed moment.
    pub fn new(now: i64) -> Self {
        Self {
            now: AtomicI64::new(now),
        }
    }

    /// Moves the clock forward.
    pub fn advance(&self, seconds: i64) {
        self.now.fetch_add(seconds, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

/// The dependencies of the handlers, over a real database.
pub fn deps(pool: &PgPool, clock: Arc<TestClock>, dev_magic_link: bool) -> Arc<Deps> {
    deps_with_tokens(
        pool,
        clock,
        dev_magic_link,
        Arc::new(PostgresTokenRepository::new(Arc::new(pool.clone()))),
    )
}

/// The sender of the tests: it records what the worker sent.
///
/// The provider of the SMS is not decided yet, and the flow has to be
/// able to prove **to whom** the message goes: the recorder is the seam.
#[derive(Debug, Default)]
pub struct RecordingSmsSender {
    sent: std::sync::Mutex<Vec<(String, String)>>,
}

impl RecordingSmsSender {
    /// The messages the worker sent, with their recipient.
    pub fn sent(&self) -> Vec<(String, String)> {
        self.sent
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// The recipient of the last message that was sent.
    pub fn last_recipient(&self) -> Option<String> {
        self.sent
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .last()
            .map(|(to, _)| to.clone())
    }

    /// The body of the last message that was sent.
    pub fn last_body(&self) -> Option<String> {
        self.sent
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .last()
            .map(|(_, body)| body.clone())
    }
}

impl SmsSender for RecordingSmsSender {
    fn send(&self, to: &str, message: &SmsMessage) -> BoxFuture<'_, Result<(), SmsError>> {
        let recipient = to.to_string();
        let body = message.body.clone();
        Box::pin(async move {
            self.sent
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push((recipient, body));
            Ok(())
        })
    }
}

/// The setup of a test.
///
/// The flows of the test run with the role of the application: the role does
/// not own the schema, so the policies of the schema apply to it and the
/// test proves what the deployment does. A test that connects with the owner
/// of the schema bypasses every policy and proves nothing.
///
/// The setup and the assertions of the test use the owner: a test has to be
/// able to see what happened, and the setup needs the context of the system
/// to create a commerce and a raffle.
pub struct Harness {
    /// The pool of the owner of the schema.
    pub owner: PgPool,
    /// The pool of the role of the application.
    pub application: PgPool,
    /// The dependencies of the handlers, over the role of the application.
    pub deps: Arc<Deps>,
    /// The SMS that the worker sent, with its recipient.
    pub sms: Arc<RecordingSmsSender>,
}

/// The setup of a test, with the role of the application.
pub async fn harness(
    clock: Arc<TestClock>,
    dev_magic_link: bool,
) -> Result<Harness, Box<dyn std::error::Error>> {
    let owner = test_pool().await?;
    let application = app_pool(&owner).await?;
    let sms = Arc::new(RecordingSmsSender::default());
    let deps = deps_with_sms(
        &application,
        clock,
        dev_magic_link,
        Arc::new(PostgresTokenRepository::new(Arc::new(application.clone()))),
        sms.clone(),
    );
    Ok(Harness {
        owner,
        application,
        deps,
        sms,
    })
}

/// The dependencies of the handlers, with the token store of the caller.
///
/// The seam exists for the races: a test that has to reproduce an
/// interleaving needs a store that pauses at a chosen point.
pub fn deps_with_tokens(
    pool: &PgPool,
    clock: Arc<TestClock>,
    dev_magic_link: bool,
    tokens: Arc<dyn TokenRepository>,
) -> Arc<Deps> {
    deps_with_sms(
        pool,
        clock,
        dev_magic_link,
        tokens,
        Arc::new(RecordingSmsSender::default()),
    )
}

/// The dependencies of the handlers, with the token store and the SMS
/// channel of the caller.
pub fn deps_with_sms(
    pool: &PgPool,
    clock: Arc<TestClock>,
    dev_magic_link: bool,
    tokens: Arc<dyn TokenRepository>,
    sms: Arc<RecordingSmsSender>,
) -> Arc<Deps> {
    let clock: Arc<dyn Clock> = clock;
    let signer = Arc::new(Hs256TokenSigner::new(
        ACCESS_SECRET.as_bytes(),
        REFRESH_SECRET.as_bytes(),
        clock.clone(),
    ));

    Arc::new(Deps {
        service: Arc::new(IdentityService {
            users: Arc::new(PostgresUserRepository::new(Arc::new(pool.clone()))),
            tokens,
            devices: Arc::new(PostgresDeviceRepository::new(Arc::new(pool.clone()))),
            consents: Arc::new(PostgresConsentRepository::new(Arc::new(pool.clone()))),
            clock: clock.clone(),
            token_source: Arc::new(RandomTokenSource),
            digest: Arc::new(Sha256Digest),
            signer: signer.clone(),
            sms,
        }),
        resolver: SessionResolver::new(signer),
        dev_magic_link,
    })
}

/// A token store that pauses the storage of a magic link.
///
/// The pause is the point of the race: two requests that ask for a link at
/// the same time have to reach the storage of the link together, or the
/// race is not reproduced and the test proves nothing.
///
/// Only the first two calls wait: the ones that come after them are the
/// retries of a request that lost the race, and a retry that waits for a
/// partner that is not coming would hang the test.
pub struct BarrierTokens {
    inner: PostgresTokenRepository,
    barrier: Arc<tokio::sync::Barrier>,
    calls: Arc<AtomicUsize>,
}

impl BarrierTokens {
    /// A store that waits for two callers before it stores a magic link.
    pub fn new(pool: &PgPool, barrier: Arc<tokio::sync::Barrier>) -> Self {
        Self {
            inner: PostgresTokenRepository::new(Arc::new(pool.clone())),
            barrier,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl TokenRepository for BarrierTokens {
    fn find_by_hash(
        &self,
        token_hash: &str,
    ) -> BoxFuture<'static, Result<Option<StoredToken>, StorageError>> {
        self.inner.find_by_hash(token_hash)
    }

    fn insert_magic_link(
        &self,
        email: &str,
        token_hash: &str,
        expires_at: i64,
    ) -> BoxFuture<'static, Result<bool, StorageError>> {
        let barrier = self.barrier.clone();
        let calls = self.calls.clone();
        let inner = self.inner.clone();
        let email = email.to_string();
        let token_hash = token_hash.to_string();
        Box::pin(async move {
            // The two requests arrive here together: the one that waits
            // leaves the store last.
            if calls.fetch_add(1, Ordering::SeqCst) < 2 {
                barrier.wait().await;
            }
            inner
                .insert_magic_link(&email, &token_hash, expires_at)
                .await
        })
    }

    fn revoke_pending_magic_links(
        &self,
        email: &str,
    ) -> BoxFuture<'static, Result<(), StorageError>> {
        self.inner.revoke_pending_magic_links(email)
    }

    fn consume_magic_link(&self, id: &str) -> BoxFuture<'static, Result<bool, StorageError>> {
        self.inner.consume_magic_link(id)
    }

    fn bind_token_user(
        &self,
        id: &str,
        user_id: &str,
    ) -> BoxFuture<'static, Result<(), StorageError>> {
        self.inner.bind_token_user(id, user_id)
    }

    fn create_family(&self, user_id: &str) -> BoxFuture<'static, Result<String, StorageError>> {
        self.inner.create_family(user_id)
    }

    fn insert_refresh(
        &self,
        user_id: &str,
        family_id: &str,
        token_hash: &str,
        expires_at: i64,
    ) -> BoxFuture<'static, Result<(), StorageError>> {
        self.inner
            .insert_refresh(user_id, family_id, token_hash, expires_at)
    }

    fn revoke_token(
        &self,
        id: &str,
        user_id: &str,
    ) -> BoxFuture<'static, Result<bool, StorageError>> {
        self.inner.revoke_token(id, user_id)
    }

    fn revoke_family(
        &self,
        family_id: &str,
        user_id: &str,
    ) -> BoxFuture<'static, Result<(), StorageError>> {
        self.inner.revoke_family(family_id, user_id)
    }

    fn revoke_all_for_user(&self, user_id: &str) -> BoxFuture<'static, Result<(), StorageError>> {
        self.inner.revoke_all_for_user(user_id)
    }
}

/// An email that no other test used.
pub fn unique_email() -> String {
    format!("participant-{}@example.test", uuid::Uuid::new_v4())
}

/// The payload of a request, as the HTTP worker hands it over.
pub fn request(body: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "body": body,
        "headers": {},
        "path_params": {},
        "query_params": {},
        "method": "POST",
    })
}

/// The payload of a request with a bearer token.
pub fn request_with_token(body: serde_json::Value, token: &str) -> serde_json::Value {
    serde_json::json!({
        "body": body,
        "headers": { "authorization": format!("Bearer {token}") },
        "path_params": {},
        "query_params": {},
        "method": "GET",
    })
}

/// The status of an answer.
pub fn status(answer: &serde_json::Value) -> u64 {
    answer["status_code"].as_u64().unwrap_or_default()
}

/// The code of the error of an answer.
pub fn error_code(answer: &serde_json::Value) -> String {
    answer["body"]["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// The data of an answer.
pub fn data(answer: &serde_json::Value) -> &serde_json::Value {
    &answer["body"]["data"]
}

/// A published raffle of a business, and a ticket of the participant in it.
///
/// The ticket is what makes the difference between a participant that can be
/// deleted and one that cannot (`FR-ID-05`).
pub async fn ticket_of(
    pool: &PgPool,
    user_id: uuid::Uuid,
) -> Result<uuid::Uuid, Box<dyn std::error::Error>> {
    let (business_id, raffle_id) = with_tenant(pool, TenantContext::system(), move |tx| {
        Box::pin(async move {
            let business_id: uuid::Uuid = sqlx::query_scalar(
                "INSERT INTO businesses (slug, name, category) VALUES ($1, 'Comercio', 'other') \
                 RETURNING id",
            )
            .bind(format!("biz-{}", uuid::Uuid::new_v4()))
            .fetch_one(&mut **tx)
            .await?;

            let raffle_id: uuid::Uuid = sqlx::query_scalar(
                "INSERT INTO raffles (business_id, slug, title, prize_name, status) \
                 VALUES ($1, $2, 'Sorteo de prueba', 'Premio', 'published') RETURNING id",
            )
            .bind(business_id)
            .bind(format!("sorteo-{}", uuid::Uuid::new_v4()))
            .fetch_one(&mut **tx)
            .await?;

            Ok::<_, sqlx::Error>((business_id, raffle_id))
        })
    })
    .await?;

    let ticket_id = with_tenant(pool, TenantContext::user(user_id), move |tx| {
        Box::pin(async move {
            let ticket_id: uuid::Uuid = sqlx::query_scalar(
                "INSERT INTO tickets (raffle_id, user_id, business_id, code, numbers_count) \
                 VALUES ($1, $2, $3, $4, 1) RETURNING id",
            )
            .bind(raffle_id)
            .bind(user_id)
            .bind(business_id)
            .bind(format!("SU-{}", uuid::Uuid::new_v4().simple()))
            .fetch_one(&mut **tx)
            .await?;
            Ok::<_, sqlx::Error>(ticket_id)
        })
    })
    .await?;

    Ok(ticket_id)
}

/// The owner of a ticket, read with the context of the system.
///
/// The tickets of a participant are invisible without a context: the
/// policies fail closed.
pub async fn ticket_owner(
    pool: &PgPool,
    ticket_id: uuid::Uuid,
) -> Result<Option<uuid::Uuid>, Box<dyn std::error::Error>> {
    let owner = with_tenant(pool, TenantContext::system(), move |tx| {
        Box::pin(async move {
            let owner: Option<uuid::Uuid> =
                sqlx::query_scalar("SELECT user_id FROM tickets WHERE id = $1")
                    .bind(ticket_id)
                    .fetch_one(&mut **tx)
                    .await?;
            Ok::<_, sqlx::Error>(owner)
        })
    })
    .await?;
    Ok(owner)
}

/// The audit rows of an entity, read with the context of the system.
pub async fn audit_rows(
    pool: &PgPool,
    entity_id: &str,
) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
    let entity_id = entity_id.to_string();
    let rows = with_tenant(pool, TenantContext::system(), move |tx| {
        Box::pin(async move {
            let rows = sqlx::query_as::<
                _,
                (String, Option<serde_json::Value>, Option<serde_json::Value>),
            >(
                "SELECT action, before, after FROM audit_log WHERE entity_id = $1 ORDER BY id",
            )
            .bind(&entity_id)
            .fetch_all(&mut **tx)
            .await?;
            Ok::<_, sqlx::Error>(rows)
        })
    })
    .await?;

    Ok(rows
        .into_iter()
        .map(|(action, before, after)| {
            serde_json::json!({ "action": action, "before": before, "after": after })
        })
        .collect())
}

/// A lock that holds the writes to the table of the participants.
///
/// The lock lets a test force two requests to the same point: the reads go
/// through and the writes wait for the lock to be released.
pub async fn lock_users(
    pool: &PgPool,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, Box<dyn std::error::Error>> {
    let mut tx = pool.begin().await?;
    sqlx::query("LOCK TABLE users IN SHARE MODE")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

/// How many requests are waiting for the lock of the table of the
/// participants.
pub async fn waiting_writers(pool: &PgPool) -> Result<i64, Box<dyn std::error::Error>> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity \
         WHERE datname = current_database() AND wait_event_type = 'Lock' \
         AND query ILIKE '%insert into users%'",
    )
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Makes sure that the role of the application exists and can read and write
/// the tables of the platform.
///
/// The role of the application does not own the schema, so the Row Level
/// Security policies apply to it: that is the second barrier of the
/// isolation (`AGENTS.md` §5.2). A test that connects with the owner of the
/// schema would pass without proving anything.
pub async fn ensure_app_role(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    APP_ROLE
        .get_or_try_init(|| async {
            let mut tx = pool.begin().await?;

            // The lock is taken for the transaction and not for the session:
            // a lock of the session is released before the commit, and the
            // grants of two sessions would collide on the same catalog row
            // ("tuple concurrently updated").
            sqlx::raw_sql("SELECT pg_advisory_xact_lock(918273645)")
                .execute(&mut *tx)
                .await?;

            sqlx::raw_sql(
                r#"
                DO $$
                BEGIN
                    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'suertudo_test_app') THEN
                        CREATE ROLE suertudo_test_app NOLOGIN;
                    END IF;
                END
                $$;
                "#,
            )
            .execute(&mut *tx)
            .await?;

            sqlx::raw_sql("GRANT USAGE ON SCHEMA public TO suertudo_test_app")
                .execute(&mut *tx)
                .await?;

            sqlx::raw_sql(
                r#"
                DO $$
                DECLARE
                    t text;
                BEGIN
                    FOREACH t IN ARRAY ARRAY[
                        'users', 'businesses', 'business_members', 'raffles', 'tickets',
                        'ticket_numbers', 'draws', 'prizes', 'theme_versions',
                        'ai_generations', 'notifications', 'consent_records',
                        'validation_codes', 'audit_log', 'auth_tokens', 'token_families',
                        'push_devices'
                    ]
                    LOOP
                        EXECUTE format(
                            'GRANT SELECT, INSERT, UPDATE ON TABLE public.%I TO suertudo_test_app',
                            t
                        );
                    END LOOP;
                END;
                $$;
                "#,
            )
            .execute(&mut *tx)
            .await?;

            sqlx::raw_sql(
                r#"
                DO $$
                DECLARE
                    s text;
                BEGIN
                    FOR s IN SELECT sequencename FROM pg_sequences WHERE schemaname = 'public' LOOP
                        EXECUTE format('GRANT USAGE ON SEQUENCE public.%I TO suertudo_test_app', s);
                    END LOOP;
                END;
                $$;
                "#,
            )
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok::<_, Box<dyn std::error::Error>>(())
        })
        .await?;
    Ok(())
}

/// A pool whose connections run with the role of the application.
///
/// The connections are the ones of the deployment: the role does not own
/// the schema and the policies of the schema apply to it.
pub async fn app_pool(pool: &PgPool) -> Result<PgPool, Box<dyn std::error::Error>> {
    ensure_app_role(pool).await?;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("SET ROLE suertudo_test_app")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url())
        .await?;
    Ok(pool)
}
