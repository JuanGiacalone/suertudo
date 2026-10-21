//! The Postgres implementation of the repositories.
//!
//! The queries are runtime checked instead of compiled: the offline cache of
//! `sqlx` lives with the crate that owns the queries, and this crate does not
//! have one. The statements are still written by hand and are the only place
//! that knows the shape of the tables.
//!
//! The rules of the product are not implemented here. They live in the schema
//! (`services/db/migrations/`): the unique index that allows a single pending
//! magic link per address, the unique index of the tokens and the Row Level
//! Security policies. This layer only uses them.

use std::sync::Arc;

use sqlx::FromRow;
use sqlx::postgres::PgPool;
use suertudo_db::{TenantContext, with_tenant};
use uuid::Uuid;

use crate::domain::errors::Refusal;
use crate::domain::types::{
    AccountStatus, ConsentScope, ConsentState, Device, NotificationPreferences, ProfilePatch, Role,
    StoredToken, TokenKind, User,
};
use crate::ports::{
    BoxFuture, ConsentRepository, DeviceRepository, StorageError, TokenRepository, UserRepository,
};

/// The PostgreSQL error codes the layer reacts to.
const UNIQUE_VIOLATION: &str = "23505";
const FOREIGN_KEY_VIOLATION: &str = "23503";
/// `object_not_in_prerequisite_state`: the trigger of the schema refuses to
/// insert a refresh token in a revoked family.
const OBJECT_NOT_IN_PREREQUISITE_STATE: &str = "55000";
/// `insufficient_privilege`: the caller is not the owner of the row.
const INSUFFICIENT_PRIVILEGE: &str = "42501";
/// The refusals of the schema that the error catalogue names.
const TICKET_LIMIT_REACHED: &str = "PT001";
const RAFFLE_NOT_OPEN: &str = "PT002";
const RAFFLE_CLOSED: &str = "PT003";
const NUMBER_TAKEN: &str = "PT004";
const TICKET_SIZE_OUT_OF_RANGE: &str = "PT005";

/// The refusal of the database that a SQLSTATE names.
///
/// The catalogue of the contract (`docs/api-contract.md` §1.4) has a code
/// and an HTTP status for each one: the mapping happens here so that no
/// refusal collapses into a `500`.
///
/// The function is public because the mapping is a contract of the crate:
/// the codes of the catalogue are what the client switches on.
pub fn refusal_of(code: &str) -> Option<Refusal> {
    match code {
        TICKET_LIMIT_REACHED => Some(Refusal::TicketLimitReached),
        RAFFLE_NOT_OPEN => Some(Refusal::RaffleNotOpen),
        RAFFLE_CLOSED => Some(Refusal::RaffleClosed),
        NUMBER_TAKEN => Some(Refusal::NumberTaken),
        TICKET_SIZE_OUT_OF_RANGE => Some(Refusal::TicketSizeOutOfRange),
        INSUFFICIENT_PRIVILEGE => Some(Refusal::RbacForbidden),
        _ => None,
    }
}

/// The storage failure of a SQLSTATE of the database.
///
/// The mapping is a contract of the crate: the client switches on the code
/// of the catalogue (`docs/api-contract.md` §1.4) and no refusal of the
/// database may collapse into a `500`.
pub fn storage_failure(code: &str, message: &str) -> StorageError {
    if code == UNIQUE_VIOLATION {
        StorageError::Conflict(message.to_string())
    } else if code == FOREIGN_KEY_VIOLATION {
        StorageError::Missing(message.to_string())
    } else if code == OBJECT_NOT_IN_PREREQUISITE_STATE {
        // The family is dead: the message is the one of the trigger of the
        // schema, and it is what tells a reuse apart from a participant that
        // does not exist.
        StorageError::FamilyRevoked(message.to_string())
    } else if let Some(refusal) = refusal_of(code) {
        // The refusal keeps its message: the answer carries the code of the
        // catalogue and the message goes to the details.
        StorageError::Refused {
            refusal,
            message: message.to_string(),
        }
    } else {
        StorageError::Backend(message.to_string())
    }
}

/// Turns a database failure into a storage failure.
fn storage_error(err: sqlx::Error) -> StorageError {
    match &err {
        sqlx::Error::Database(db) => {
            let code = db.code().unwrap_or_default();
            storage_failure(code.as_ref(), db.message())
        }
        _ => StorageError::Backend(err.to_string()),
    }
}

impl From<sqlx::Error> for StorageError {
    fn from(err: sqlx::Error) -> Self {
        storage_error(err)
    }
}

/// Parses an identifier that comes from the outside.
fn parse_id(value: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(value).map_err(|_| StorageError::Backend(format!("invalid identifier {value}")))
}

/// The columns of a participant.
#[derive(Debug, FromRow)]
struct UserRow {
    id: Uuid,
    email: String,
    display_name: String,
    role: String,
    status: String,
    phone_e164: Option<String>,
    locale: String,
    timezone: String,
    marketing_opt_in: bool,
    notification_preferences: serde_json::Value,
}

impl From<UserRow> for User {
    fn from(row: UserRow) -> Self {
        User {
            id: row.id.to_string(),
            email: row.email,
            display_name: row.display_name,
            role: Role::parse(&row.role).unwrap_or(Role::Participant),
            status: AccountStatus::parse(&row.status).unwrap_or(AccountStatus::Active),
            phone_e164: row.phone_e164,
            locale: row.locale,
            timezone: row.timezone,
            marketing_opt_in: row.marketing_opt_in,
            notification_preferences: notification_preferences(
                &row.notification_preferences,
                row.marketing_opt_in,
            ),
        }
    }
}

/// The preferences of the participant, as they are stored.
///
/// The push and the email channels live in the column of the preferences.
/// The marketing one also lives in `users.marketing_opt_in`, and that column
/// is the one the rest of the platform reads: the two are written together
/// and the column wins when they are read.
fn notification_preferences(
    stored: &serde_json::Value,
    marketing_opt_in: bool,
) -> NotificationPreferences {
    NotificationPreferences {
        push: stored
            .get("push")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        email: stored
            .get("email")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        marketing: marketing_opt_in,
    }
}

/// The participants, in Postgres.
#[derive(Debug)]
pub struct PostgresUserRepository {
    pool: Arc<PgPool>,
}

impl PostgresUserRepository {
    /// Builds the repository over a pool.
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

impl UserRepository for PostgresUserRepository {
    fn find_by_email(&self, email: &str) -> BoxFuture<'static, Result<Option<User>, StorageError>> {
        let pool = self.pool.clone();
        let email = email.to_string();
        Box::pin(async move {
            // The login has no tenant context: the participant does not
            // exist yet, or is not authenticated yet. The function of the
            // schema resolves the address and returns the row and nothing
            // else.
            let row = sqlx::query_as::<_, UserRow>(
                "SELECT id, email, display_name, role, status, phone_e164, locale, timezone, \
                 marketing_opt_in, notification_preferences FROM find_user_by_email($1)",
            )
            .bind(&email)
            .fetch_optional(&*pool)
            .await
            .map_err(storage_error)?;
            Ok(row.map(User::from))
        })
    }

    fn find_by_id(&self, id: &str) -> BoxFuture<'static, Result<Option<User>, StorageError>> {
        let pool = self.pool.clone();
        let id = id.to_string();
        Box::pin(async move {
            let user_id = parse_id(&id)?;
            // The row of the participant is read with the context of the
            // participant: the policy of the schema lets a participant read
            // its own row and nothing else.
            let row = with_tenant(&pool, TenantContext::user(user_id), move |tx| {
                Box::pin(async move {
                    let row = sqlx::query_as::<_, UserRow>(
                        "SELECT id, email, display_name, role, status, phone_e164, locale, \
                         timezone, marketing_opt_in, notification_preferences FROM users \
                         WHERE id = $1 AND deleted_at IS NULL",
                    )
                    .bind(user_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(storage_error)?;
                    Ok::<_, StorageError>(row)
                })
            })
            .await?;
            Ok(row.map(User::from))
        })
    }

    fn create(
        &self,
        email: &str,
        display_name: &str,
    ) -> BoxFuture<'static, Result<User, StorageError>> {
        let pool = self.pool.clone();
        let email = email.to_string();
        let display_name = display_name.to_string();
        Box::pin(async move {
            // The registration has no tenant context either: the
            // participant does not exist yet. The function of the schema
            // writes the row and returns its identifier.
            let id: Uuid = sqlx::query_scalar("SELECT register_user($1, $2)")
                .bind(&email)
                .bind(&display_name)
                .fetch_one(&*pool)
                .await
                .map_err(storage_error)?;

            // The row comes back with the same function that resolves the
            // address: the defaults of the schema are part of it.
            let row = sqlx::query_as::<_, UserRow>(
                "SELECT id, email, display_name, role, status, phone_e164, locale, timezone, \
                 marketing_opt_in, notification_preferences FROM find_user_by_email($1)",
            )
            .bind(&email)
            .fetch_optional(&*pool)
            .await
            .map_err(storage_error)?;

            let Some(row) = row else {
                return Err(StorageError::Missing(format!(
                    "the participant {email} was not registered"
                )));
            };
            debug_assert_eq!(row.id, id);
            Ok(User::from(row))
        })
    }

    fn update(
        &self,
        id: &str,
        patch: ProfilePatch,
    ) -> BoxFuture<'static, Result<Option<User>, StorageError>> {
        let pool = self.pool.clone();
        let id = id.to_string();
        Box::pin(async move {
            let user_id = parse_id(&id)?;
            let phone_provided = patch.phone_e164.is_some();
            let phone = patch.phone_e164.clone().flatten();
            let marketing = patch.notification_preferences.map(|prefs| prefs.marketing);
            // The preferences are stored as they arrive: the column of the
            // schema holds the three channels of `docs/api-contract.md`
            // §3.6, and the marketing one also goes to the column the rest
            // of the platform reads.
            let preferences = patch.notification_preferences.map(|prefs| {
                serde_json::json!({
                    "push": prefs.push,
                    "email": prefs.email,
                    "marketing": prefs.marketing,
                })
            });

            // The profile is written with the context of the participant:
            // the policy of the schema lets a participant update its own
            // row and nothing else. The audit row travels in the same
            // transaction: a second connection would wait for the lock of
            // the participant that the transaction already holds.
            let updated: Option<(UserRow, Option<serde_json::Value>)> =
                with_tenant(&pool, TenantContext::user(user_id), move |tx| {
                Box::pin(async move {
                    let before = sqlx::query_as::<_, UserRow>(
                        "SELECT id, email, display_name, role, status, phone_e164, locale, \
                         timezone, marketing_opt_in, notification_preferences FROM users \
                         WHERE id = $1 AND deleted_at IS NULL",
                    )
                    .bind(user_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(storage_error)?;

                    let after = sqlx::query_as::<_, UserRow>(
                        r#"
                        UPDATE users SET
                            display_name = COALESCE($2::text, display_name),
                            phone_e164 = CASE WHEN $3::boolean THEN $4::text ELSE phone_e164 END,
                            locale = COALESCE($5::text, locale),
                            timezone = COALESCE($6::text, timezone),
                            marketing_opt_in = COALESCE($7::boolean, marketing_opt_in),
                            notification_preferences = COALESCE($8::jsonb, notification_preferences),
                            updated_at = now()
                        WHERE id = $1 AND deleted_at IS NULL
                        RETURNING id, email, display_name, role, status, phone_e164, locale,
                            timezone, marketing_opt_in, notification_preferences
                        "#,
                    )
                    .bind(user_id)
                    .bind(patch.display_name)
                    .bind(phone_provided)
                    .bind(phone)
                    .bind(patch.locale)
                    .bind(patch.timezone)
                    .bind(marketing)
                    .bind(preferences)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(storage_error)?;

                    let Some(after) = after else {
                        return Ok::<_, StorageError>(None);
                    };

                    Ok::<_, StorageError>(Some((
                        after,
                        before.as_ref().map(audit_snapshot),
                    )))
                })
            })
            .await?;

            let Some((row, before)) = updated else {
                return Ok(None);
            };

            // The profile carries personal data: the change is written to
            // the append only log, so a deletion of the phone is still
            // auditable. The log keeps the fact of the change, never the
            // value of the personal data.
            write_audit(
                &pool,
                user_id,
                "user.profile.updated",
                before,
                Some(audit_snapshot(&row)),
            )
            .await?;

            Ok(Some(User::from(row)))
        })
    }

    fn anonymise(&self, id: &str) -> BoxFuture<'static, Result<(), StorageError>> {
        let pool = self.pool.clone();
        let id = id.to_string();
        Box::pin(async move {
            let user_id = parse_id(&id)?;
            // The anonymisation runs with the context of the participant:
            // the policy of the schema lets a participant write its own row
            // and nothing else, and the function of the schema checks that
            // the tickets it anonymises belong to it.
            let original_email = with_tenant(&pool, TenantContext::user(user_id), move |tx| {
                Box::pin(async move {
                    // The address before the anonymisation: the tokens carry
                    // it too, and the account has to leave no trace of it.
                    let original_email: Option<String> = sqlx::query_scalar(
                        "SELECT email FROM users WHERE id = $1 AND deleted_at IS NULL",
                    )
                    .bind(user_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(storage_error)?;

                    let Some(original_email) = original_email else {
                        // The account is already gone.
                        return Ok::<_, StorageError>(None);
                    };

                    // The address becomes a placeholder that cannot collide:
                    // the identifier is part of it, and the index on the
                    // lowercased address stays satisfied.
                    sqlx::query(
                        r#"
                        UPDATE users SET
                            email = 'deleted+' || id::text || '@suertudo.invalid',
                            display_name = 'Deleted user',
                            phone_e164 = NULL,
                            marketing_opt_in = false,
                            status = 'deleted',
                            deleted_at = now(),
                            updated_at = now()
                        WHERE id = $1 AND deleted_at IS NULL
                        "#,
                    )
                    .bind(user_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(storage_error)?;

                    // The tickets survive with a null owner and the prizes
                    // keep theirs (`FR-ID-05`).
                    //
                    // The write goes through the function of the schema and
                    // not through a plain UPDATE: an anonymised ticket stops
                    // being visible to the participant that anonymised it,
                    // and Postgres refuses a row that the writer cannot see
                    // anymore. The function is `SECURITY DEFINER` and runs
                    // as the owner of the table.
                    sqlx::query("SELECT anonymise_user_tickets($1)")
                        .bind(user_id)
                        .execute(&mut **tx)
                        .await
                        .map_err(storage_error)?;

                    // The tokens of the participant carry the address as
                    // well. They die with the account and their address goes
                    // with it.
                    sqlx::query(
                        r#"
                        UPDATE auth_tokens SET
                            email = 'deleted+' || id::text || '@suertudo.invalid',
                            consumed_at = COALESCE(consumed_at, now()),
                            revoked_at = COALESCE(revoked_at, now())
                        WHERE user_id = $1
                        "#,
                    )
                    .bind(user_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(storage_error)?;

                    // The pending links of the address belong to nobody yet:
                    // a participant cannot write a token that has no owner.
                    // The function of the schema dies them anyway, and a
                    // pending link of the old address could otherwise create
                    // a new account with it.
                    sqlx::query("SELECT revoke_pending_magic_links($1)")
                        .bind(&original_email)
                        .execute(&mut **tx)
                        .await
                        .map_err(storage_error)?;

                    // The device tokens go with the account: the token of a
                    // device is a personal datum and a credential (`FR-ID-05`).
                    sqlx::query("SELECT anonymise_user_devices($1)")
                        .bind(user_id)
                        .execute(&mut **tx)
                        .await
                        .map_err(storage_error)?;

                    Ok::<_, StorageError>(Some(original_email))
                })
            })
            .await?;

            let Some(original_email) = original_email else {
                return Ok(());
            };

            // The address of the tokens that belong to nobody is the last
            // trace of the participant: they are the pending links of the
            // address, dead already, and a participant cannot write them.
            // The platform does it, with the context of the system, and only
            // for the address that is being deleted.
            with_tenant(&pool, TenantContext::system(), move |tx| {
                Box::pin(async move {
                    sqlx::query(
                        r#"
                        UPDATE auth_tokens SET
                            email = 'deleted+' || id::text || '@suertudo.invalid'
                        WHERE email = $1 AND user_id IS NULL
                        "#,
                    )
                    .bind(&original_email)
                    .execute(&mut **tx)
                    .await
                    .map_err(storage_error)?;

                    // The audit keeps the fact of the anonymisation, never
                    // the data that was removed. The row has no business:
                    // the platform writes it.
                    sqlx::query(
                        r#"
                        INSERT INTO audit_log (actor_user_id, actor_type, action, entity_type, entity_id, before, after)
                        VALUES ($1, 'user', 'user.account.anonymised', 'user', $1::text, $2, $3)
                        "#,
                    )
                    .bind(user_id)
                    .bind(serde_json::json!({ "personal_data": "present" }))
                    .bind(serde_json::json!({ "personal_data": "anonymised" }))
                    .execute(&mut **tx)
                    .await
                    .map_err(storage_error)?;
                    Ok::<_, StorageError>(())
                })
            })
            .await
        })
    }
}

/// The snapshot of a participant for the audit log.
///
/// The log keeps the fact of the change and not the value of the personal
/// data: a phone number is not written to the log, only whether there was
/// one (`docs/PRD.md` §11.1).
fn audit_snapshot(row: &UserRow) -> serde_json::Value {
    serde_json::json!({
        "display_name": !row.display_name.is_empty(),
        "phone_e164": row.phone_e164.is_some(),
        "locale": !row.locale.is_empty(),
        "timezone": !row.timezone.is_empty(),
        "marketing_opt_in": row.marketing_opt_in,
    })
}

/// Writes one row to the append only audit log.
///
/// The row of a participant has no business, and the policy of the schema
/// lets the platform write it: the context is the one of the system.
///
/// The write happens in its own transaction, after the change it records:
/// a row of the log that holds a participant as its actor takes a lock on
/// that participant, and the transaction of the change already holds it.
async fn write_audit(
    pool: &PgPool,
    actor_user_id: Uuid,
    action: &str,
    before: Option<serde_json::Value>,
    after: Option<serde_json::Value>,
) -> Result<(), StorageError> {
    let action = action.to_string();
    with_tenant(pool, TenantContext::system(), move |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO audit_log (actor_user_id, actor_type, action, entity_type, entity_id, before, after)
                VALUES ($1, 'user', $2, 'user', $1::text, $3, $4)
                "#,
            )
            .bind(actor_user_id)
            .bind(&action)
            .bind(before)
            .bind(after)
            .execute(&mut **tx)
            .await
            .map_err(storage_error)?;
            Ok::<_, StorageError>(())
        })
    })
    .await
}

/// The columns of a token.
#[derive(Debug, FromRow)]
struct TokenRow {
    id: Uuid,
    user_id: Option<Uuid>,
    email: String,
    kind: String,
    family_id: Option<Uuid>,
    expires_at: chrono::DateTime<chrono::Utc>,
    consumed_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The tokens, in Postgres.
///
/// Only the hash of a token is stored. If the database leaks, the tokens it
/// holds cannot be used.
#[derive(Debug, Clone)]
pub struct PostgresTokenRepository {
    pool: Arc<PgPool>,
}

impl PostgresTokenRepository {
    /// Builds the repository over a pool.
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

impl TokenRepository for PostgresTokenRepository {
    fn find_by_hash(
        &self,
        token_hash: &str,
    ) -> BoxFuture<'static, Result<Option<StoredToken>, StorageError>> {
        let pool = self.pool.clone();
        let token_hash = token_hash.to_string();
        Box::pin(async move {
            let bytes = hex::decode(&token_hash)
                .map_err(|_| StorageError::Backend("the token hash is not hexadecimal".into()))?;
            // The login has no tenant context: the token is the credential
            // and its hash is the secret. The function of the schema
            // resolves the hash and returns the row and nothing else.
            let row = sqlx::query_as::<_, TokenRow>(
                r#"
                SELECT id, user_id, email, kind, family_id, expires_at, consumed_at, revoked_at
                FROM find_token_by_hash($1)
                "#,
            )
            .bind(bytes)
            .fetch_optional(&*pool)
            .await
            .map_err(storage_error)?;

            Ok(row.map(|row| StoredToken {
                id: row.id.to_string(),
                user_id: row.user_id.map(|id| id.to_string()),
                email: row.email,
                kind: if row.kind == "magic_link" {
                    TokenKind::MagicLink
                } else {
                    TokenKind::Refresh
                },
                family_id: row.family_id.map(|id| id.to_string()),
                expires_at: row.expires_at.timestamp(),
                consumed: row.consumed_at.is_some(),
                revoked: row.revoked_at.is_some(),
            }))
        })
    }

    fn insert_magic_link(
        &self,
        email: &str,
        token_hash: &str,
        expires_at: i64,
    ) -> BoxFuture<'static, Result<bool, StorageError>> {
        let pool = self.pool.clone();
        let email = email.to_string();
        let token_hash = token_hash.to_string();
        Box::pin(async move {
            let bytes = hex::decode(&token_hash)
                .map_err(|_| StorageError::Backend("the token hash is not hexadecimal".into()))?;
            // The emission of a link is the function of the schema: the
            // requester is not authenticated yet, and the address is a
            // personal datum (`docs/api-contract.md` §3.1).
            //
            // The schema allows a single pending link per address, and the
            // conflict is not an error the caller can see: the endpoint
            // answers `202` whether the address exists or not, and a race
            // between two requests is the same thing (`FR-ID-01`). The
            // caller retries.
            match sqlx::query_scalar::<_, Uuid>(
                "SELECT insert_magic_link($1, $2, to_timestamp($3))",
            )
            .bind(&email)
            .bind(bytes)
            .bind(expires_at as f64)
            .fetch_one(&*pool)
            .await
            {
                Ok(_) => Ok(true),
                // The pending slot of the address is taken: the caller
                // retries after cancelling the link that is pending.
                Err(sqlx::Error::Database(db))
                    if db.code().as_deref() == Some(UNIQUE_VIOLATION) =>
                {
                    Ok(false)
                }
                Err(err) => Err(storage_error(err)),
            }
        })
    }

    fn revoke_pending_magic_links(
        &self,
        email: &str,
    ) -> BoxFuture<'static, Result<(), StorageError>> {
        let pool = self.pool.clone();
        let email = email.to_string();
        Box::pin(async move {
            // The link that is replaced is consumed: the partial unique index
            // of the schema allows a single pending link per address. The
            // login has no tenant context, so the write goes through the
            // function of the schema.
            sqlx::query("SELECT revoke_pending_magic_links($1)")
                .bind(&email)
                .execute(&*pool)
                .await
                .map_err(storage_error)?;
            Ok(())
        })
    }

    fn consume_magic_link(&self, id: &str) -> BoxFuture<'static, Result<bool, StorageError>> {
        let pool = self.pool.clone();
        let id = id.to_string();
        Box::pin(async move {
            // The consumption is atomic: the check and the write are one
            // statement, and the loser of the race updates zero rows. The
            // participant is bound to the link afterwards, so an account is
            // never created for a link that somebody else won.
            //
            // The participant is not authenticated yet, so there is no
            // tenant context: the function of the schema does the write.
            let consumed: bool = sqlx::query_scalar("SELECT consume_magic_link($1)")
                .bind(parse_id(&id)?)
                .fetch_one(&*pool)
                .await
                .map_err(storage_error)?;
            Ok(consumed)
        })
    }

    fn bind_token_user(
        &self,
        id: &str,
        user_id: &str,
    ) -> BoxFuture<'static, Result<(), StorageError>> {
        let pool = self.pool.clone();
        let id = id.to_string();
        let user_id = user_id.to_string();
        Box::pin(async move {
            // The link is bound to the participant that won it. The write
            // goes through the function of the schema: the participant is
            // not authenticated yet.
            let bound: bool = sqlx::query_scalar("SELECT bind_magic_link_user($1, $2)")
                .bind(parse_id(&id)?)
                .bind(parse_id(&user_id)?)
                .fetch_one(&*pool)
                .await
                .map_err(storage_error)?;
            if !bound {
                return Err(StorageError::Missing(format!(
                    "the link {id} was not consumed"
                )));
            }
            Ok(())
        })
    }

    fn create_family(&self, user_id: &str) -> BoxFuture<'static, Result<String, StorageError>> {
        let pool = self.pool.clone();
        let user_id = user_id.to_string();
        Box::pin(async move {
            let participant = parse_id(&user_id)?;
            // The family exists before its first token: the foreign key of
            // `auth_tokens` rejects a token of a family that does not exist,
            // and `token_families.revoked_at` is the death of the family.
            //
            // The family belongs to the participant: the policy of the
            // schema admits the row only with the context of its owner.
            let id: Uuid = with_tenant(&pool, TenantContext::user(participant), move |tx| {
                Box::pin(async move {
                    let id: Uuid = sqlx::query_scalar(
                        "INSERT INTO token_families (user_id) VALUES ($1) RETURNING id",
                    )
                    .bind(participant)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(storage_error)?;
                    Ok::<_, StorageError>(id)
                })
            })
            .await?;
            Ok(id.to_string())
        })
    }

    fn insert_refresh(
        &self,
        user_id: &str,
        family_id: &str,
        token_hash: &str,
        expires_at: i64,
    ) -> BoxFuture<'static, Result<(), StorageError>> {
        let pool = self.pool.clone();
        let user_id = user_id.to_string();
        let family_id = family_id.to_string();
        let token_hash = token_hash.to_string();
        Box::pin(async move {
            let bytes = hex::decode(&token_hash)
                .map_err(|_| StorageError::Backend("the token hash is not hexadecimal".into()))?;
            let participant = parse_id(&user_id)?;
            let family = parse_id(&family_id)?;
            // The address of the participant comes from the row of the
            // participant: `auth_tokens.email` does not accept a null.
            //
            // Two failures live here and they look the same from the
            // outside: the participant does not exist (zero rows) and the
            // family is dead (the trigger of the schema refuses the insert
            // with the SQLSTATE `55000`). The second one is the detection of
            // a reuse (`FR-ID-04`).
            //
            // The token belongs to the participant: the policy of the schema
            // admits it only with the context of its owner.
            let inserted = with_tenant(&pool, TenantContext::user(participant), move |tx| {
                Box::pin(async move {
                    let result = sqlx::query(
                        r#"
                        INSERT INTO auth_tokens (user_id, email, kind, token_hash, family_id, expires_at)
                        SELECT id, email, 'refresh', $2, $3, to_timestamp($4) FROM users WHERE id = $1
                        "#,
                    )
                    .bind(participant)
                    .bind(bytes)
                    .bind(family)
                    .bind(expires_at as f64)
                    .execute(&mut **tx)
                    .await
                    .map_err(storage_error)?;
                    Ok::<_, StorageError>(result.rows_affected() == 1)
                })
            })
            .await?;

            if !inserted {
                return Err(StorageError::Missing(format!(
                    "the participant {user_id} does not exist"
                )));
            }
            Ok(())
        })
    }

    fn revoke_token(
        &self,
        id: &str,
        user_id: &str,
    ) -> BoxFuture<'static, Result<bool, StorageError>> {
        let pool = self.pool.clone();
        let id = id.to_string();
        let user_id = user_id.to_string();
        Box::pin(async move {
            let participant = parse_id(&user_id)?;
            let token = parse_id(&id)?;
            // A rotated token keeps its row: the row is what proves that the
            // token came back, and that is what takes the family down.
            //
            // The token belongs to the participant: the policy of the schema
            // admits the write only with the context of its owner.
            let revoked = with_tenant(&pool, TenantContext::user(participant), move |tx| {
                Box::pin(async move {
                    let result = sqlx::query(
                        "UPDATE auth_tokens SET revoked_at = now()                          WHERE id = $1 AND revoked_at IS NULL",
                    )
                    .bind(token)
                    .execute(&mut **tx)
                    .await
                    .map_err(storage_error)?;
                    Ok::<_, StorageError>(result.rows_affected() == 1)
                })
            })
            .await?;

            // Zero rows means somebody else revoked it first. This call is a
            // reuse, not a rotation, and the service has to treat it as one.
            Ok(revoked)
        })
    }

    fn revoke_family(
        &self,
        family_id: &str,
        user_id: &str,
    ) -> BoxFuture<'static, Result<(), StorageError>> {
        let pool = self.pool.clone();
        let family_id = family_id.to_string();
        let user_id = user_id.to_string();
        Box::pin(async move {
            let participant = parse_id(&user_id)?;
            let family = parse_id(&family_id)?;
            // The family is the one that dies: `token_families.revoked_at` is
            // the death, and the trigger of the schema takes the tokens of
            // the family down with it. The death does not depend on the
            // order of the writes: a successor that is inserted after it is
            // refused by the trigger of the schema.
            //
            // The family belongs to the participant: the policy of the schema
            // admits the write only with the context of its owner.
            with_tenant(&pool, TenantContext::user(participant), move |tx| {
                Box::pin(async move {
                    sqlx::query(
                        "UPDATE token_families SET revoked_at = now(), revoke_reason = 'revoked' \
                         WHERE id = $1 AND revoked_at IS NULL",
                    )
                    .bind(family)
                    .execute(&mut **tx)
                    .await
                    .map_err(storage_error)?;

                    // The tokens of the family die with it. The trigger does
                    // it too, but a family that does not exist (a row from
                    // before the migration) has no trigger to fire.
                    sqlx::query(
                        "UPDATE auth_tokens SET revoked_at = now() \
                         WHERE family_id = $1 AND revoked_at IS NULL",
                    )
                    .bind(family)
                    .execute(&mut **tx)
                    .await
                    .map_err(storage_error)?;
                    Ok::<_, StorageError>(())
                })
            })
            .await
        })
    }

    fn revoke_all_for_user(&self, user_id: &str) -> BoxFuture<'static, Result<(), StorageError>> {
        let pool = self.pool.clone();
        let user_id = user_id.to_string();
        Box::pin(async move {
            let participant = parse_id(&user_id)?;
            // The families of the participant die first: the trigger of the
            // schema revokes their tokens, and a family that is dead refuses
            // the successors that a race could try to insert.
            //
            // The tokens belong to the participant: the policy of the schema
            // admits the writes only with the context of its owner.
            with_tenant(&pool, TenantContext::user(participant), move |tx| {
                Box::pin(async move {
                    sqlx::query(
                        "UPDATE token_families SET revoked_at = now(), \
                         revoke_reason = 'account_deleted' \
                         WHERE user_id = $1 AND revoked_at IS NULL",
                    )
                    .bind(participant)
                    .execute(&mut **tx)
                    .await
                    .map_err(storage_error)?;

                    sqlx::query(
                        "UPDATE auth_tokens SET revoked_at = now() \
                         WHERE user_id = $1 AND revoked_at IS NULL",
                    )
                    .bind(participant)
                    .execute(&mut **tx)
                    .await
                    .map_err(storage_error)?;
                    Ok::<_, StorageError>(())
                })
            })
            .await
        })
    }
}

/// The columns of a push device.
#[derive(Debug, FromRow)]
struct DeviceRow {
    id: Uuid,
    platform: String,
    token: String,
    app_version: Option<String>,
    active: bool,
}

/// The push devices, in Postgres.
#[derive(Debug)]
pub struct PostgresDeviceRepository {
    pool: Arc<PgPool>,
}

impl PostgresDeviceRepository {
    /// Builds the repository over a pool.
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

impl DeviceRepository for PostgresDeviceRepository {
    fn register(
        &self,
        user_id: &str,
        platform: &str,
        token: &str,
        app_version: Option<&str>,
    ) -> BoxFuture<'static, Result<Device, StorageError>> {
        let pool = self.pool.clone();
        let user_id = user_id.to_string();
        let platform = platform.to_string();
        let token = token.to_string();
        let app_version = app_version.map(str::to_string);
        Box::pin(async move {
            let user_uuid = parse_id(&user_id)?;
            // The registration of a device is the function of the schema:
            // the token of the device is a credential and the row belongs to
            // its owner (`docs/api-contract.md` §3.8).
            // The function of the schema checks who calls it: the device
            // belongs to the participant of the context.
            let id: Uuid = with_tenant(&pool, TenantContext::user(user_uuid), move |tx| {
                Box::pin(async move {
                    let id: Uuid =
                        sqlx::query_scalar("SELECT register_push_device($1, $2, $3, $4)")
                            .bind(user_uuid)
                            .bind(&platform)
                            .bind(&token)
                            .bind(&app_version)
                            .fetch_one(&mut **tx)
                            .await
                            .map_err(storage_error)?;
                    Ok::<_, StorageError>(id)
                })
            })
            .await?;

            // The row comes back with the context of its owner: the token of
            // a device is a credential and nobody else sees it.
            let row = with_tenant(&pool, TenantContext::user(user_uuid), move |tx| {
                Box::pin(async move {
                    let row = sqlx::query_as::<_, DeviceRow>(
                        "SELECT id, platform, token, app_version, active FROM push_devices \
                         WHERE id = $1",
                    )
                    .bind(id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(storage_error)?;
                    Ok::<_, StorageError>(row)
                })
            })
            .await?;

            let Some(row) = row else {
                return Err(StorageError::Missing(format!(
                    "the device {id} was not registered"
                )));
            };

            Ok(Device {
                id: row.id.to_string(),
                platform: row.platform,
                token: row.token,
                app_version: row.app_version,
                active: row.active,
            })
        })
    }

    fn deactivate(
        &self,
        user_id: &str,
        device_id: &str,
    ) -> BoxFuture<'static, Result<bool, StorageError>> {
        let pool = self.pool.clone();
        let user_id = user_id.to_string();
        let device_id = device_id.to_string();
        Box::pin(async move {
            let owner = parse_id(&user_id)?;
            let device = parse_id(&device_id)?;
            // The device belongs to the participant: the policy of the
            // schema admits the write only with the context of its owner.
            let deactivated = with_tenant(&pool, TenantContext::user(owner), move |tx| {
                Box::pin(async move {
                    let result = sqlx::query(
                        "UPDATE push_devices SET active = false \
                         WHERE id = $1 AND user_id = $2 AND active = true",
                    )
                    .bind(device)
                    .bind(owner)
                    .execute(&mut **tx)
                    .await
                    .map_err(storage_error)?;
                    Ok::<_, StorageError>(result.rows_affected() == 1)
                })
            })
            .await?;
            Ok(deactivated)
        })
    }
}

/// The columns of a consent.
#[derive(Debug, FromRow)]
struct ConsentRow {
    business_id: Uuid,
    scope: String,
    granted: bool,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// The consents, in Postgres.
///
/// The table is append only: the state of a scope is the last row written for
/// it. The queries run with the participant as the tenant, so the Row Level
/// Security policy of the table applies.
#[derive(Debug)]
pub struct PostgresConsentRepository {
    pool: Arc<PgPool>,
}

impl PostgresConsentRepository {
    /// Builds the repository over a pool.
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

impl ConsentRepository for PostgresConsentRepository {
    fn list(&self, user_id: &str) -> BoxFuture<'static, Result<Vec<ConsentState>, StorageError>> {
        let pool = self.pool.clone();
        let user_id = user_id.to_string();
        Box::pin(async move {
            let user_uuid = parse_id(&user_id)?;
            let rows = with_tenant(&pool, TenantContext::user(user_uuid), move |tx| {
                Box::pin(async move {
                    let rows = sqlx::query_as::<_, ConsentRow>(
                        r#"
                        SELECT business_id, scope, granted, created_at
                        FROM consent_records
                        WHERE user_id = $1
                        ORDER BY business_id, scope, created_at DESC
                        "#,
                    )
                    .bind(user_uuid)
                    .fetch_all(&mut **tx)
                    .await
                    .map_err(storage_error)?;
                    Ok::<_, StorageError>(rows)
                })
            })
            .await?;

            // The last row of a scope wins: the rows before it are the
            // history of the consent.
            let mut states: Vec<ConsentState> = Vec::new();
            for row in rows {
                let Some(scope) = consent_scope(&row.scope) else {
                    continue;
                };
                let business_id = row.business_id.to_string();
                let already = states
                    .iter()
                    .any(|state| state.business_id == business_id && state.scope == scope);
                if already {
                    continue;
                }
                states.push(ConsentState {
                    business_id,
                    scope,
                    granted: row.granted,
                    updated_at: row.created_at.timestamp(),
                });
            }
            Ok(states)
        })
    }

    fn record(
        &self,
        user_id: &str,
        business_id: &str,
        scope: ConsentScope,
        granted: bool,
        user_agent: Option<&str>,
    ) -> BoxFuture<'static, Result<(), StorageError>> {
        let pool = self.pool.clone();
        let user_id = user_id.to_string();
        let business_id = business_id.to_string();
        let scope = scope.as_str().to_string();
        let user_agent = user_agent.map(str::to_string);
        Box::pin(async move {
            let user_uuid = parse_id(&user_id)?;
            let business_uuid = parse_id(&business_id)?;
            // The consent is the function of the schema: the row carries the
            // business of the other side and the participant cannot write
            // it on its own (`docs/api-contract.md` §3.9).
            with_tenant(&pool, TenantContext::user(user_uuid), move |tx| {
                Box::pin(async move {
                    let id: Uuid =
                        sqlx::query_scalar("SELECT record_consent($1, $2, $3, $4, 'settings', $5)")
                            .bind(user_uuid)
                            .bind(business_uuid)
                            .bind(&scope)
                            .bind(granted)
                            .bind(&user_agent)
                            .fetch_one(&mut **tx)
                            .await
                            .map_err(storage_error)?;
                    let _ = id;
                    Ok::<_, StorageError>(())
                })
            })
            .await
        })
    }
}

/// The scope stored in the database.
fn consent_scope(value: &str) -> Option<ConsentScope> {
    match value {
        "share_marketing" => Some(ConsentScope::ShareMarketing),
        "share_contact" => Some(ConsentScope::ShareContact),
        _ => None,
    }
}
