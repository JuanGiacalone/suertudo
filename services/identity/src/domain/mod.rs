//! Pure identity rules.
//!
//! Nothing in this module may depend on `iii_sdk`, `sqlx`, the system clock
//! or the system randomness. Anything the domain needs from the outside
//! arrives through `crate::ports`.

pub mod errors;
pub mod profile;
pub mod service;
pub mod session;
pub mod types;

pub use errors::{IdentityError, Refusal};
pub use service::IdentityService;
pub use types::{
    AccessClaims, AccountStatus, ConsentScope, ConsentState, Device, DeviceRegistration,
    DeviceRequest, DeviceVerification, MagicLinkClaims, MagicLinkIssued, NotificationPreferences,
    ProfilePatch, RefreshClaims, Role, Session, StoredToken, TokenKind, User,
};
