//! Execution of the draw (`docs/data-model.md` §6.3).
//!
//! The unique constraint on `draws.raffle_id` is the idempotency barrier: two
//! workers that reach the draw at the same time produce one row and the other
//! one gets nothing back.

use serde_json::Value;
use sqlx::postgres::PgPool;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::RepositoryError;
use crate::pool::TenantContext;

/// Everything needed to open a draw.
#[derive(Debug, Clone)]
pub struct DrawSeed {
    pub raffle_id: Uuid,
    pub business_id: Uuid,
    /// hex(SHA-256(seed)): the public commitment.
    pub seed_commitment: String,
    /// The same seed, encrypted with AES-256-GCM.
    pub seed_ciphertext: Vec<u8>,
    /// The derivation formula, stored with the draw so a public verification
    /// stays exact even if the formula changes later.
    pub derivation_spec: Value,
    pub pool_size: i32,
}

/// Opens the draw of a raffle, or returns `None` if it already exists.
pub async fn create_pending_draw(
    tx: &mut Transaction<'_, Postgres>,
    seed: &DrawSeed,
) -> Result<Option<Uuid>, RepositoryError> {
    Ok(sqlx::query_scalar!(
        r#"
        INSERT INTO draws (raffle_id, business_id, seed_commitment, seed_ciphertext,
                           algorithm, derivation_spec, pool_size, status)
        VALUES ($1, $2, $3, $4, 'hmac-sha256/v1', $5, $6, 'pending')
        ON CONFLICT (raffle_id) DO NOTHING
        RETURNING id
        "#,
        seed.raffle_id,
        seed.business_id,
        seed.seed_commitment,
        seed.seed_ciphertext,
        seed.derivation_spec,
        seed.pool_size,
    )
    .fetch_optional(&mut **tx)
    .await?)
}

/// Pool-level wrapper for [`create_pending_draw`].
pub async fn create_pending_draw_in_tenant(
    pool: &PgPool,
    ctx: TenantContext,
    seed: &DrawSeed,
) -> Result<Option<Uuid>, RepositoryError> {
    let seed = seed.clone();
    crate::pool::with_tenant(pool, ctx, move |tx| {
        Box::pin(async move { create_pending_draw(tx, &seed).await })
    })
    .await
}
