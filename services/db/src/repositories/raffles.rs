//! Read queries over `raffles` (`docs/data-model.md` §6.1, §6.5 and §6.6).

use sqlx::postgres::PgPool;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use super::RepositoryError;
use crate::pool::TenantContext;

/// How many numbers are still available in a raffle.
///
/// `numbers_reserved` is denormalised on purpose: the feed shows "quedan N
/// números" in every card and a `COUNT` per card does not scale.
#[derive(Debug, Clone, Copy, FromRow)]
pub struct NumbersAvailability {
    pub id: Uuid,
    pub number_pool_size: i32,
    pub numbers_reserved: i32,
    pub numbers_available: i32,
}

/// A raffle that is due to be drawn.
#[derive(Debug, Clone, Copy, FromRow)]
pub struct RaffleToDraw {
    pub id: Uuid,
    pub business_id: Uuid,
    pub number_pool_size: i32,
    pub prize_quantity: i32,
}

/// Aggregated numbers for the business panel.
#[derive(Debug, Clone, Copy, FromRow)]
pub struct RaffleMetrics {
    pub active_tickets: i64,
    pub winning_tickets: i64,
    pub unique_participants: i64,
    pub numbers_reserved: i64,
}

/// Numbers still available in a raffle (§6.1).
pub async fn numbers_availability(
    tx: &mut Transaction<'_, Postgres>,
    raffle_id: Uuid,
) -> Result<NumbersAvailability, RepositoryError> {
    // The available count is an expression, and Postgres reports every
    // expression as nullable even when the operands are not.
    let row = sqlx::query!(
        r#"
        SELECT r.id,
               r.number_pool_size,
               r.numbers_reserved,
               r.number_pool_size - r.numbers_reserved AS numbers_available
        FROM raffles r
        WHERE r.id = $1
        "#,
        raffle_id
    )
    .fetch_one(&mut **tx)
    .await?;

    Ok(NumbersAvailability {
        id: row.id,
        number_pool_size: row.number_pool_size,
        numbers_reserved: row.numbers_reserved,
        numbers_available: row.numbers_available.unwrap_or_default(),
    })
}

/// Raffles that are closed and whose draw time already passed (§6.5).
///
/// The worker asks this question every minute; the partial index
/// `raffles_draws_at_idx` serves it.
pub async fn raffles_to_draw(
    tx: &mut Transaction<'_, Postgres>,
    limit: i64,
) -> Result<Vec<RaffleToDraw>, RepositoryError> {
    Ok(sqlx::query_as!(
        RaffleToDraw,
        r#"
        SELECT id, business_id, number_pool_size, prize_quantity
        FROM raffles
        WHERE status = 'closed'
          AND draws_at <= now()
          AND NOT EXISTS (SELECT 1 FROM draws d WHERE d.raffle_id = raffles.id)
        ORDER BY draws_at
        LIMIT $1
        "#,
        limit
    )
    .fetch_all(&mut **tx)
    .await?)
}

/// Metrics of one raffle for the business panel (§6.6).
pub async fn raffle_metrics(
    tx: &mut Transaction<'_, Postgres>,
    raffle_id: Uuid,
) -> Result<RaffleMetrics, RepositoryError> {
    // The aggregates are expressions: Postgres reports them as nullable.
    let row = sqlx::query!(
        r#"
        SELECT
            count(*) FILTER (WHERE status = 'reserved')    AS active_tickets,
            count(*) FILTER (WHERE status = 'winner')      AS winning_tickets,
            count(DISTINCT user_id)                        AS unique_participants,
            (SELECT count(*) FROM ticket_numbers tn WHERE tn.raffle_id = $1) AS numbers_reserved
        FROM tickets
        WHERE raffle_id = $1 AND status <> 'cancelled'
        "#,
        raffle_id
    )
    .fetch_one(&mut **tx)
    .await?;

    Ok(RaffleMetrics {
        active_tickets: row.active_tickets.unwrap_or_default(),
        winning_tickets: row.winning_tickets.unwrap_or_default(),
        unique_participants: row.unique_participants.unwrap_or_default(),
        numbers_reserved: row.numbers_reserved.unwrap_or_default(),
    })
}

/// Pool-level wrapper for [`numbers_availability`].
///
/// The public feed reads raffles without a tenant, so the context is the
/// system one: a published raffle is visible to anybody.
pub async fn numbers_availability_in_tenant(
    pool: &PgPool,
    ctx: TenantContext,
    raffle_id: Uuid,
) -> Result<NumbersAvailability, RepositoryError> {
    crate::pool::with_tenant(pool, ctx, |tx| {
        Box::pin(async move { numbers_availability(tx, raffle_id).await })
    })
    .await
}
