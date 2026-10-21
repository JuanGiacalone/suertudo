//! Atomic reservation of a ticket and its numbers (`docs/data-model.md` §6.2).
//!
//! The reservation is a single transaction on purpose: the ticket, its numbers
//! and the denormalised counters of the raffle move together or not at all.
//! The `FOR UPDATE` on the raffle row serialises concurrent reservations of
//! the same raffle, which is what lets the automatic pick choose free numbers
//! without a race.

use sqlx::postgres::PgPool;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use super::RepositoryError;
use crate::pool::TenantContext;

/// The raffle row as the reservation needs to see it.
#[derive(Debug, Clone, FromRow)]
pub struct RaffleForReservation {
    pub id: Uuid,
    pub business_id: Uuid,
    pub status: String,
    pub number_pool_size: i32,
    pub numbers_per_ticket_min: i32,
    pub numbers_per_ticket_max: i32,
}

/// The outcome of a successful reservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reservation {
    pub ticket_id: Uuid,
    pub numbers: Vec<i32>,
}

/// Reserves a ticket with its numbers inside an open transaction.
///
/// The caller owns the transaction: the reservation has to commit together with
/// whatever the handler does next (notifications, outbox events). Use
/// [`reserve_ticket_in_tenant`] when the caller does not need to.
///
/// The write is the function of the schema (`reserve_ticket`): the
/// reservation is made by a participant, who is not the owner of the raffle,
/// and the write policies of Row Level Security only let the owner of the
/// raffle write its row. The function locks the raffle row, which is the
/// serialisation point: two participants reserving at the same time wait for
/// each other instead of picking the same number.
pub async fn reserve_ticket(
    tx: &mut Transaction<'_, Postgres>,
    raffle_id: Uuid,
    user_id: Uuid,
    code: &str,
    numbers: &[i32],
    auto_picked: bool,
) -> Result<Reservation, RepositoryError> {
    // The application's own validation, with its own errors. The function
    // checks the same facts again: this is the check that reads well, the
    // one in the database is the guarantee.
    let raffle = sqlx::query_as!(
        RaffleForReservation,
        r#"
        SELECT id, business_id, status, number_pool_size,
               numbers_per_ticket_min, numbers_per_ticket_max
        FROM raffles
        WHERE id = $1 AND deleted_at IS NULL
        "#,
        raffle_id
    )
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(RepositoryError::RaffleNotFound { raffle_id })?;

    if raffle.status != "published" {
        return Err(RepositoryError::RaffleNotOpen {
            raffle_id,
            status: raffle.status.clone(),
        });
    }

    let requested = numbers.len() as i32;
    if requested < raffle.numbers_per_ticket_min || requested > raffle.numbers_per_ticket_max {
        return Err(RepositoryError::NumbersOutOfRange {
            requested: numbers.len(),
            min: raffle.numbers_per_ticket_min,
            max: raffle.numbers_per_ticket_max,
        });
    }

    for number in numbers {
        if *number < 0 || *number >= raffle.number_pool_size {
            return Err(RepositoryError::NumberOutOfPool {
                number: *number,
                pool_size: raffle.number_pool_size,
            });
        }
    }

    let ticket_id: Option<Uuid> = sqlx::query_scalar!(
        "SELECT reserve_ticket($1, $2, $3, $4, $5)",
        raffle_id,
        user_id,
        code,
        numbers,
        auto_picked,
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|err| reservation_error(err, raffle_id, numbers.len()))?;

    let ticket_id = match ticket_id {
        Some(id) => id,
        None => return Err(RepositoryError::RaffleNotFound { raffle_id }),
    };

    Ok(Reservation {
        ticket_id,
        numbers: numbers.to_vec(),
    })
}

/// The refusals of the function of the schema, mapped to the errors of the
/// application.
///
/// The SQLSTATEs are the ones of `migrations/0028_tenant_functions.sql`:
/// `PT001` is the limit of tickets, `PT002` and `PT003` son el sorteo que no
/// está abierto, `PT004` es el número tomado y `PT005` el tamaño de la
/// reserva.
fn reservation_error(err: sqlx::Error, raffle_id: Uuid, requested: usize) -> RepositoryError {
    let code = err
        .as_database_error()
        .and_then(|db| db.code())
        .map(|code| code.into_owned());

    match code.as_deref() {
        Some("PT001") => RepositoryError::TicketLimitReached { raffle_id },
        Some("PT002") => RepositoryError::RaffleNotOpen {
            raffle_id,
            status: "draft".to_string(),
        },
        Some("PT003") => RepositoryError::RaffleNotOpen {
            raffle_id,
            status: "closed".to_string(),
        },
        Some("PT004") => RepositoryError::NumberTaken { raffle_id },
        Some("PT005") => RepositoryError::NumbersOutOfRange {
            requested,
            min: 0,
            max: 0,
        },
        _ => RepositoryError::Db(err),
    }
}

/// Reserves a ticket inside a transaction that carries the tenant context.
pub async fn reserve_ticket_in_tenant(
    pool: &PgPool,
    ctx: TenantContext,
    raffle_id: Uuid,
    user_id: Uuid,
    code: &str,
    numbers: &[i32],
    auto_picked: bool,
) -> Result<Reservation, RepositoryError> {
    let code = code.to_string();
    let numbers = numbers.to_vec();
    crate::pool::with_tenant(pool, ctx, move |tx| {
        Box::pin(async move {
            reserve_ticket(tx, raffle_id, user_id, &code, &numbers, auto_picked).await
        })
    })
    .await
}
