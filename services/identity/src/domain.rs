//! Pure identity rules.
//!
//! Nothing in this module may depend on `iii_sdk`, `sqlx`, the system clock
//! or the system randomness. Anything the domain needs from the outside
//! arrives through `crate::ports`.
