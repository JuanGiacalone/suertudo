//! The draw: the mechanism that decides who wins.
//!
//! # The promise of this crate
//!
//! A participant who was not there, a business that suspects the outcome, and
//! a third party with nothing but the public values of the raffle have to be
//! able to reach **the same number**. That is the whole product.
//!
//! The formula is:
//!
//! ```text
//! winner_index = u64_be(HMAC-SHA256(seed, "suertudo/v1|raffle:{id}|draw:{i}")[0..8]) % pool_size
//! ```
//!
//! The seed is committed at the moment the raffle is published, and revealed
//! when the raffle is drawn. Nobody can choose the seed after seeing who
//! reserved which numbers, and everybody can recompute the winner.
//!
//! # The rules this module follows
//!
//! - **No clock, no randomness, no database.** The seed and the raffle id
//!   arrive as arguments. The module is a pure function of them.
//! - **The formula is frozen.** The golden test in `derivation.rs` pins the
//!   value `144` for a known seed. Changing the formula makes that test
//!   fail on purpose, and the PRD and the data model have to change with it.
//! - **The public string is part of the contract.** The message template
//!   with the raffle id and the draw index is what a third party types into
//!   their own implementation. It is frozen with the formula.

pub mod derivation;

pub use derivation::{
    DerivationSpec, MESSAGE_TEMPLATE, derive_winner_index, public_derivation_spec,
};
