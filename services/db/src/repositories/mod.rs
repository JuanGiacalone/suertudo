//! Repository layer: the queries of `docs/data-model.md` §6, written as
//! `sqlx` functions.
//!
//! Every function that reads tenant data takes a transaction, because the
//! tenant context (`app.business_id` / `app.user_id`) only lives inside one.
//! The wrappers in this module run the closure of `pool::with_tenant`, so a
//! caller cannot accidentally read tenant rows without a context.
//!
//! The invariants of the product are **not** implemented here. They live in the
//! schema (`services/db/migrations/`): the unique constraint on
//! `ticket_numbers (raffle_id, number)`, the unique constraint on
//! `draws.raffle_id`, the partial unique index on `tickets (raffle_id,
//! user_id)` and the `tickets_business_match_trg` trigger. This layer only
//! uses them.

pub mod consents;
pub mod draws;
pub mod raffles;
pub mod tickets;

use sqlx::Error;
use thiserror::Error;
use uuid::Uuid;

pub use consents::{ConsentedContact, consented_contacts};
pub use draws::{DrawSeed, create_pending_draw};
pub use raffles::{
    NumbersAvailability, RaffleMetrics, RaffleToDraw, numbers_availability, raffle_metrics,
    raffles_to_draw,
};
pub use tickets::{RaffleForReservation, Reservation, reserve_ticket, reserve_ticket_in_tenant};

/// Everything a repository can fail with.
///
/// Database errors are kept apart from the business rules: a unique violation
/// is the database doing its job, not a bug in the caller.
#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("database error: {0}")]
    Db(#[from] Error),

    #[error("raffle {raffle_id} is not open for reservations (status = {status})")]
    RaffleNotOpen { raffle_id: Uuid, status: String },

    #[error("requested {requested} numbers, the raffle allows between {min} and {max}")]
    NumbersOutOfRange {
        requested: usize,
        min: i32,
        max: i32,
    },

    #[error("number {number} is outside the pool of {pool_size} numbers")]
    NumberOutOfPool { number: i32, pool_size: i32 },

    #[error("raffle {raffle_id} was not found")]
    RaffleNotFound { raffle_id: Uuid },

    #[error("ticket {ticket_id} was not found")]
    TicketNotFound { ticket_id: Uuid },

    #[error("the participant reached the limit of tickets of raffle {raffle_id}")]
    TicketLimitReached { raffle_id: Uuid },

    #[error("a number of raffle {raffle_id} is already reserved")]
    NumberTaken { raffle_id: Uuid },
}
