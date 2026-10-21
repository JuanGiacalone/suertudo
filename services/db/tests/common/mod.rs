#![allow(dead_code)]
//! Shared helpers for the integration tests.
//!
//! Every test runs against a real Postgres: the invariants of the product are
//! guarantees of the database, and a test that does not touch a database
//! cannot prove them.

use std::error::Error;

use futures_util::future::BoxFuture;
use sqlx::postgres::PgPool;
use sqlx::{Postgres, Transaction};
use suertudo_db::pool;
use tokio::sync::OnceCell;
use uuid::Uuid;

/// The setup of the test database, done once per test binary.
///
/// The migrations and the role of the application touch the catalog, and two
/// concurrent `CREATE TABLE IF NOT EXISTS` or `GRANT`s over the same catalog
/// row fail with "tuple concurrently updated". The tests run in parallel, so
/// the setup has to happen once.
static SETUP: OnceCell<()> = OnceCell::const_new();

/// Connection string of the test database.
pub fn database_url() -> String {
    std::env::var("DATABASE_URL_TEST")
        .unwrap_or_else(|_| "postgres://suertudo:suertudo@127.0.0.1:5432/suertudo_test".to_string())
}

/// A pool with the migrations applied and the role of the application ready.
///
/// Every test gets its own pool, because every test has its own runtime. The
/// setup of the database is shared, and it happens only once.
///
/// The tests do not clean up: every test creates rows with its own random
/// identifiers, so the tests can run in parallel without interfering.
pub async fn test_pool() -> Result<PgPool, Box<dyn Error>> {
    let pool = pool::connect(&database_url(), 10).await?;
    SETUP
        .get_or_try_init(|| async {
            pool::migrate(&pool).await?;
            ensure_app_role(&pool).await?;
            Ok::<_, Box<dyn Error>>(())
        })
        .await?;
    Ok(pool)
}

/// True when the error is a unique violation of the given constraint.
pub fn is_unique_violation(err: &sqlx::Error, constraint: &str) -> bool {
    match err {
        sqlx::Error::Database(db_err) => {
            db_err.code().as_deref() == Some("23505") && db_err.constraint() == Some(constraint)
        }
        _ => false,
    }
}

/// True when the error carries the given message.
pub fn has_message(err: &sqlx::Error, message: &str) -> bool {
    match err {
        sqlx::Error::Database(db_err) => db_err.message().contains(message),
        _ => false,
    }
}

/// A ticket code that no other test used.
pub fn unique_code() -> String {
    format!("TK{}", &Uuid::new_v4().simple().to_string()[..8])
}

/// Creates a business and returns its id.
pub async fn seed_business(pool: &PgPool) -> Result<Uuid, Box<dyn Error>> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO businesses (slug, name, category) VALUES ($1, $2, 'other') RETURNING id",
    )
    .bind(format!("biz-{}", Uuid::new_v4()))
    .bind("Comercio de prueba")
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Creates a participant and returns its id.
pub async fn seed_user(pool: &PgPool) -> Result<Uuid, Box<dyn Error>> {
    let id: Uuid =
        sqlx::query_scalar("INSERT INTO users (email, display_name) VALUES ($1, $2) RETURNING id")
            .bind(format!("user-{}@example.test", Uuid::new_v4()))
            .bind("Participante de prueba")
            .fetch_one(pool)
            .await?;
    Ok(id)
}

/// Creates a raffle with the given status and returns its id.
pub async fn seed_raffle(
    pool: &PgPool,
    business_id: Uuid,
    status: &str,
) -> Result<Uuid, Box<dyn Error>> {
    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO raffles (business_id, slug, title, prize_name, status, number_pool_size,
                             numbers_per_ticket_min, numbers_per_ticket_max, max_tickets_per_user)
        VALUES ($1, $2, 'Sorteo de prueba', 'Premio', $3, 1000, 1, 5, 1)
        RETURNING id
        "#,
    )
    .bind(business_id)
    .bind(format!("sorteo-{}", Uuid::new_v4()))
    .bind(status)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Creates a ticket for a raffle and returns its id.
pub async fn seed_ticket(
    pool: &PgPool,
    raffle_id: Uuid,
    business_id: Uuid,
    user_id: Uuid,
) -> Result<Uuid, Box<dyn Error>> {
    let code = unique_code();
    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO tickets (raffle_id, user_id, business_id, code, numbers_count)
        VALUES ($1, $2, $3, $4, 1)
        RETURNING id
        "#,
    )
    .bind(raffle_id)
    .bind(user_id)
    .bind(business_id)
    .bind(code)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// A slug that no other test used.
pub fn unique_slug() -> String {
    format!("sorteo-{}", &Uuid::new_v4().simple().to_string()[..8])
}

/// Runs a closure in a transaction that carries the role of the application
/// and the tenant context.
///
/// The role is the one that does not own the tables: the owner bypasses Row
/// Level Security by design, so a test that connects as the owner proves
/// nothing about the deployment.
///
/// A closure that returns an error leaves the transaction aborted: the helper
/// rolls it back instead of committing, and the error reaches the caller.
pub async fn with_app_role<'c, F, R>(
    pool: &'c PgPool,
    business_id: Option<Uuid>,
    user_id: Option<Uuid>,
    f: F,
) -> Result<R, sqlx::Error>
where
    F: for<'t> FnOnce(&'t mut Transaction<'_, Postgres>) -> BoxFuture<'t, Result<R, sqlx::Error>>,
{
    let mut tx = pool.begin().await?;

    sqlx::query("SET LOCAL ROLE suertudo_test_app")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.business_id', $1, true)")
        .bind(business_id.map(|id| id.to_string()).unwrap_or_default())
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.map(|id| id.to_string()).unwrap_or_default())
        .execute(&mut *tx)
        .await?;

    match f(&mut tx).await {
        Ok(out) => {
            tx.commit().await?;
            Ok(out)
        }
        Err(err) => {
            let _ = tx.rollback().await;
            Err(err)
        }
    }
}

/// Runs a write with the role of the application and the context of a
/// business, and returns the error the database raised.
///
/// The transaction is rolled back: the write was refused, and nothing has to
/// survive it.
pub async fn rejected_write<'c, F>(
    pool: &'c PgPool,
    business_id: Uuid,
    f: F,
) -> Result<sqlx::Error, Box<dyn Error>>
where
    F: for<'t> FnOnce(&'t mut Transaction<'_, Postgres>) -> BoxFuture<'t, Result<(), sqlx::Error>>
        + Send
        + 'c,
{
    let mut tx = pool.begin().await?;

    sqlx::query("SET LOCAL ROLE suertudo_test_app")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.business_id', $1, true)")
        .bind(business_id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind("")
        .execute(&mut *tx)
        .await?;

    let outcome = f(&mut tx).await;
    let _ = tx.rollback().await;

    match outcome {
        Ok(()) => Err("the write of the rows of another business was accepted".into()),
        Err(err) => Ok(err),
    }
}

/// Runs a closure in a transaction with the tenant context of a participant.
///
/// The reservation is made by a participant, and the function of the schema
/// is bound to the caller: the transaction carries its context.
pub async fn with_participant<'c, F, R, E>(pool: &'c PgPool, user_id: Uuid, f: F) -> Result<R, E>
where
    F: for<'t> FnOnce(&'t mut Transaction<'_, Postgres>) -> BoxFuture<'t, Result<R, E>>,
    E: From<sqlx::Error>,
{
    let mut tx = pool.begin().await?;

    sqlx::query("SELECT set_config('app.business_id', '', true)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;

    match f(&mut tx).await {
        Ok(out) => {
            tx.commit().await?;
            Ok(out)
        }
        Err(err) => {
            let _ = tx.rollback().await;
            Err(err)
        }
    }
}

/// Runs a closure with the role of the application and the context of a
/// participant.
///
/// The participant is the owner of its own rows (the tickets, the tokens,
/// the devices), and nobody else's.
pub async fn with_app_participant<'c, F, R>(
    pool: &'c PgPool,
    user_id: Uuid,
    f: F,
) -> Result<R, sqlx::Error>
where
    F: for<'t> FnOnce(&'t mut Transaction<'_, Postgres>) -> BoxFuture<'t, Result<R, sqlx::Error>>,
{
    let mut tx = pool.begin().await?;

    sqlx::query("SET LOCAL ROLE suertudo_test_app")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.business_id', '', true)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;

    match f(&mut tx).await {
        Ok(out) => {
            tx.commit().await?;
            Ok(out)
        }
        Err(err) => {
            let _ = tx.rollback().await;
            Err(err)
        }
    }
}

/// Runs a closure with the role of the application and the context of the
/// platform.
///
/// El contexto de sistema es el marcador explícito (`'system'`): un GUC sin
/// definir o vacío es "no hay contexto" y falla cerrado.
pub async fn with_app_system<'c, F, R>(pool: &'c PgPool, f: F) -> Result<R, sqlx::Error>
where
    F: for<'t> FnOnce(&'t mut Transaction<'_, Postgres>) -> BoxFuture<'t, Result<R, sqlx::Error>>,
{
    let mut tx = pool.begin().await?;

    sqlx::query("SET LOCAL ROLE suertudo_test_app")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.business_id', 'system', true)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.user_id', 'system', true)")
        .execute(&mut *tx)
        .await?;

    match f(&mut tx).await {
        Ok(out) => {
            tx.commit().await?;
            Ok(out)
        }
        Err(err) => {
            let _ = tx.rollback().await;
            Err(err)
        }
    }
}

/// True when the error is the refusal of the limit of tickets per raffle.
///
/// The trigger of the schema raises `PT001` (`migrations/0031`).
pub fn is_limit_reached(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db| db.code())
        .map(|code| code == "PT001")
        .unwrap_or(false)
}

/// Creates the role the application would connect with.
///
/// The role is not the owner of the tables: the owner bypasses Row Level
/// Security, and the point of the RLS tests is to check the behaviour of a
/// connection that does not.
///
/// The grants update the catalog, and a catalog row that another transaction
/// is updating at the same time (the autovacuum, for example) makes the
/// `GRANT` fail with "tuple concurrently updated". The setup retries, because
/// the alternative is a test that fails for a reason that has nothing to do
/// with what it proves.
pub async fn ensure_app_role(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let mut last_error: Option<Box<dyn Error>> = None;

    for attempt in 1..=5 {
        match grant_to_app_role(pool).await {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_error = Some(err);
                tokio::time::sleep(std::time::Duration::from_millis(50 * attempt)).await;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "the role of the application is not ready".into()))
}

/// The grants of the role of the application.
async fn grant_to_app_role(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let mut tx = pool.begin().await?;

    // The lock has to be the transactional one. A session level lock is
    // released when the statement ends, which is before the commit: two test
    // binaries would then run the grants at the same time and collide on the
    // same catalog row with "tuple concurrently updated".
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

    // The application role reads and writes. The Row Level Security
    // policies are the barrier: the grants say what the role *can* do, the
    // policies say whose rows it can touch.
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
                'outbox', 'push_devices'
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

    sqlx::raw_sql("SELECT pg_advisory_unlock(918273645)")
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}
