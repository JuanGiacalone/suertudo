//! Connection pool and tenant context.
//!
//! The Row Level Security policies in `docs/data-model.md` §7.3 are the
//! second barrier of the multi-tenant isolation. They read `app.business_id`
//! and `app.user_id` from the session, so every transaction has to set them.

use futures_util::future::BoxFuture;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

/// Postgres connection pool.
pub type PgConnection = PgPool;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("could not reach the database: {0}")]
    Connect(#[from] sqlx::Error),
    #[error("migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
}

/// The identity of the request that owns a transaction.
///
/// `business_id` and `user_id` are set with `SET LOCAL`, so they only live
/// for the duration of the transaction.
#[derive(Debug, Clone, Copy)]
pub struct TenantContext {
    pub business_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
}

impl TenantContext {
    /// A context with no tenant: used by jobs that are not tenant-scoped,
    /// such as the one that closes expired raffles.
    pub fn system() -> Self {
        Self {
            business_id: None,
            user_id: None,
        }
    }

    /// A context owned by a participant.
    pub fn user(user_id: Uuid) -> Self {
        Self {
            business_id: None,
            user_id: Some(user_id),
        }
    }

    /// A context owned by a business.
    pub fn business(business_id: Uuid) -> Self {
        Self {
            business_id: Some(business_id),
            user_id: None,
        }
    }
}

/// Builds a connection pool from `DATABASE_URL`.
pub async fn connect(database_url: &str, max_connections: u32) -> Result<PgPool, DbError> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await
        .map_err(DbError::Connect)
}

/// Runs a closure inside a transaction that carries the tenant context.
///
/// The `SET LOCAL` calls are what make the RLS policies match. Without them
/// the policies fail closed and the closure sees no rows, which is the
/// intended behaviour: a missing context is a bug, not an open door.
///
/// The closure receives the transaction so that every query in it runs under
/// the same context and the same snapshot.
pub async fn with_tenant<'c, F, R, E>(pool: &'c PgPool, ctx: TenantContext, f: F) -> Result<R, E>
where
    F: for<'t> FnOnce(&'t mut Transaction<'_, Postgres>) -> BoxFuture<'t, Result<R, E>>,
    E: From<sqlx::Error>,
{
    let mut tx = pool.begin().await?;

    if let Some(business_id) = ctx.business_id {
        sqlx::query("SELECT set_config('app.business_id', $1, true)")
            .bind(business_id.to_string())
            .execute(&mut *tx)
            .await?;
    }
    if let Some(user_id) = ctx.user_id {
        sqlx::query("SELECT set_config('app.user_id', $1, true)")
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?;
    }

    let out = f(&mut tx).await?;

    tx.commit().await?;
    Ok(out)
}

/// Applies every pending migration.
pub async fn migrate(pool: &PgPool) -> Result<(), DbError> {
    crate::MIGRATOR.run(pool).await.map_err(DbError::Migration)
}
