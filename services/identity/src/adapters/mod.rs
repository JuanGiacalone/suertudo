//! Concrete implementations of the ports in `crate::ports`.
//!
//! Postgres, Argon2, JWT, the system clock and the system CSPRNG.

pub mod clock;
pub mod crypto;
pub mod postgres;
pub mod sms;
pub mod tokens;

pub use clock::SystemClock;
pub use crypto::{Argon2PasswordHasher, RandomTokenSource, Sha256Digest};
pub use postgres::{
    PostgresConsentRepository, PostgresDeviceRepository, PostgresTokenRepository,
    PostgresUserRepository, refusal_of, storage_failure,
};
pub use sms::LoggingSmsSender;
pub use tokens::Hs256TokenSigner;
