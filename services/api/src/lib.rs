//! The HTTP surface of the Suertudo backend.
//!
//! The API worker owns the endpoints that are not owned by a domain worker.
//! The identity endpoints live in the identity worker; this crate owns the
//! public surface (health, public raffle pages) and the shared plumbing.
//!
//! # The rule that does not bend
//!
//! **`business_id` is never read from a request body or a path parameter.**
//! It is derived from the session token by the authorization middleware and
//! injected into the handler context. A handler that receives a
//! `business_id` from the caller is a bug, and the isolation test in
//! `tests/isolation.rs` exists to catch it.

pub mod handlers;
pub mod health;
pub mod middleware;
