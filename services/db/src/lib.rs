//! Database access for Suertudo.
//!
//! Owns the connection pool, the migration runner and the helpers that every
//! other crate needs to talk to Postgres safely.
//!
//! Two rules live here, and they are the reason this crate exists:
//!
//! 1. **Every transaction sets the tenant context.** `with_tenant` runs a
//!    closure inside a transaction that has `app.business_id` and
//!    `app.user_id` set, so the Row Level Security policies of
//!    `docs/data-model.md` §7.3 apply. A transaction without that context
//!    reads nothing: the policies fail closed.
//! 2. **Migrations are the only way the schema changes.** Files in
//!    `migrations/` are applied in order and never edited once merged.
//!
//! This crate is infrastructure: it talks to Postgres on purpose. It has no
//! `domain/` module, and the rules of `AGENTS.md` §5.1 apply to the crates
//! that consume it, not to this one.

pub mod pool;
pub mod repositories;

pub use pool::{DbError, PgConnection, TenantContext, with_tenant};
pub use repositories::RepositoryError;

/// Migrations embedded in the binary, applied in filename order.
///
/// The directory is relative to this crate's `Cargo.toml`.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
