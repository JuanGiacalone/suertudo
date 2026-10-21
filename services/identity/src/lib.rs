//! Identity: registration, sessions and the authorization context.
//!
//! # The layering rule
//!
//! This crate has three layers and the boundary between them is not
//! negotiable:
//!
//! ```text
//! src/domain/   pure business logic. No iii, no sqlx, no clock, no randomness.
//! src/ports.rs  traits for everything the domain needs from the outside.
//! src/adapters/ implementations of those traits (postgres, clock, rng).
//! src/handlers.rs  the iii surface: functions and their schemas.
//! src/main.rs   the worker: registration of functions and triggers.
//! ```
//!
//! `domain/` and `ports/` **must not** depend on `iii_sdk` or `sqlx`. That
//! is what makes the business rules testable without a database, without a
//! network and without an engine, and it is what keeps the backend portable
//! if the orchestrator changes (`docs/architecture.md` ADR-002).
//!
//! The architecture test in `services/api/tests/architecture.rs` enforces
//! the boundary mechanically.
//!
//! # What lives here
//!
//! - The magic link of the participant: one use, fifteen minutes
//!   (`FR-ID-01`).
//! - The session: an access token of fifteen minutes and a refresh token of
//!   thirty days that rotates (`FR-ID-04`).
//! - The profile, the push devices and the consents of the participant
//!   (`FR-ID-05`).
//! - The middleware that resolves the session and injects the identity of
//!   the caller (`FR-ID-07`).

pub mod adapters;
pub mod config;
pub mod domain;
pub mod handlers;
pub mod http;
pub mod middleware;
pub mod ports;

pub use config::{Config, ConfigError, Environment};
pub use domain::errors::IdentityError;
pub use domain::service::IdentityService;
pub use handlers::Deps;
pub use middleware::SessionResolver;
