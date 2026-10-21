//! What the domain needs from the outside world.
//!
//! Every trait here is implemented in `crate::adapters`. The domain only ever
//! sees these traits, never the concrete implementations.
//!
//! The traits return a boxed future with a `'static` lifetime on purpose: an
//! implementation owns what it needs (the pool is an `Arc`, the arguments are
//! copied) instead of borrowing the caller. That keeps the objects usable as
//! `Arc<dyn Trait>` without fighting the borrow checker at every call site.

use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

use crate::domain::errors::Refusal;
use crate::domain::types::{
    AccessClaims, ConsentScope, ConsentState, Device, DeviceRequest, DeviceVerification,
    MagicLinkClaims, ProfilePatch, RefreshClaims, StoredToken, User,
};

/// A future that can be stored in a trait object.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The wall clock.
///
/// The domain never reads the system clock directly: the expiry rules are
/// only testable if the clock is injected (AGENTS.md §8).
pub trait Clock: Send + Sync {
    /// The current time, in seconds since the Unix epoch.
    fn now(&self) -> i64;
}

/// The source of the unguessable values the identity needs.
pub trait TokenSource: Send + Sync {
    /// A random secret, in hexadecimal. Used for the magic link token.
    fn next_secret(&self) -> String;
    /// A random identifier, in its UUID textual form. Used for the family of
    /// a refresh token.
    fn next_id(&self) -> String;
    /// A code of `digits` digits, for a verification the participant reads
    /// on a phone. It comes from the CSPRNG of the kernel: a code that can
    /// be guessed is not a verification.
    fn next_code(&self, digits: usize) -> String;
}

/// A one way digest.
pub trait Digest: Send + Sync {
    /// The SHA-256 of the input, in hexadecimal.
    fn sha256_hex(&self, input: &str) -> String;
}

/// The failures of a cryptographic operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CryptoError {
    /// The credential is not valid: wrong signature, wrong shape, unknown key.
    #[error("the credential is not valid")]
    Invalid,
    /// The credential is valid but expired.
    #[error("the credential expired")]
    Expired,
    /// The cryptographic operation failed for an unexpected reason.
    #[error("the cryptographic operation failed: {0}")]
    Internal(String),
}

/// The hashing of passwords.
///
/// No endpoint uses it yet: the business login (`FR-ID-02`) reuses it, and
/// the participant does not have a password at all.
pub trait PasswordHasher: Send + Sync {
    /// Hashes a password.
    fn hash(&self, password: &str) -> Result<String, CryptoError>;
    /// Checks a password against a hash.
    fn verify(&self, password: &str, hash: &str) -> Result<bool, CryptoError>;
}

/// The signer of the three kinds of token the identity issues.
///
/// The access token and the refresh token are signed with different keys, so
/// a stolen refresh token cannot be used as an access token and the other way
/// around. Every token carries its kind in the `typ` claim and the verifier
/// rejects the tokens of another kind.
pub trait TokenSigner: Send + Sync {
    /// Signs the claims of an access token.
    fn sign_access(&self, claims: &AccessClaims) -> Result<String, CryptoError>;
    /// Verifies an access token and returns its claims.
    fn verify_access(&self, token: &str) -> Result<AccessClaims, CryptoError>;
    /// Signs the claims of a refresh token.
    fn sign_refresh(&self, claims: &RefreshClaims) -> Result<String, CryptoError>;
    /// Verifies a refresh token and returns its claims.
    fn verify_refresh(&self, token: &str) -> Result<RefreshClaims, CryptoError>;
    /// Signs the claims of a magic link.
    fn sign_magic_link(&self, claims: &MagicLinkClaims) -> Result<String, CryptoError>;
    /// Verifies a magic link and returns its claims.
    fn verify_magic_link(&self, token: &str) -> Result<MagicLinkClaims, CryptoError>;
}

/// The body of an SMS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmsMessage {
    /// The text of the message.
    pub body: String,
}

impl SmsMessage {
    /// A message with its body.
    pub fn new(body: impl Into<String>) -> Self {
        Self { body: body.into() }
    }
}

/// The failure of the SMS channel.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SmsError {
    /// The provider refused the message.
    #[error("the SMS provider refused the message: {0}")]
    Refused(String),
    /// The channel failed for a reason the provider does not explain.
    #[error("the SMS channel failed: {0}")]
    Channel(String),
}

/// The SMS channel. The provider is not decided yet: the flow is built
/// against this port so that integrating one is a matter of writing an
/// implementation.
pub trait SmsSender: Send + Sync {
    fn send(&self, to: &str, message: &SmsMessage) -> BoxFuture<'_, Result<(), SmsError>>;
}

/// The failures of the storage layer.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StorageError {
    /// The database refused the operation for an unexpected reason.
    #[error("the storage layer failed: {0}")]
    Backend(String),
    /// The database refused the write because it conflicts with the current
    /// state. The uniqueness rules of the schema live in the database.
    #[error("the storage layer rejected the write: {0}")]
    Conflict(String),
    /// The row the write refers to does not exist.
    #[error("the storage layer found no row: {0}")]
    Missing(String),
    /// The family of the refresh token is dead.
    ///
    /// The database refuses to insert a token in a revoked family, and it
    /// says so with a different code than the one of a missing row. The
    /// difference matters: a missing participant is a `404`, and a dead
    /// family is the detection of a reuse (`FR-ID-04`).
    #[error("the refresh token family is revoked: {0}")]
    FamilyRevoked(String),
    /// The database refused the operation with a code of the error
    /// catalogue (`docs/api-contract.md` §1.4).
    ///
    /// The refusal keeps its code and its message: the worker answers with
    /// the code and the client switches on it.
    #[error("the storage layer refused the write: {message}")]
    Refused { refusal: Refusal, message: String },
}

/// The participants.
pub trait UserRepository: Send + Sync {
    /// Looks a participant up by email, case insensitively.
    fn find_by_email(&self, email: &str) -> BoxFuture<'static, Result<Option<User>, StorageError>>;
    /// Looks a participant up by identifier.
    fn find_by_id(&self, id: &str) -> BoxFuture<'static, Result<Option<User>, StorageError>>;
    /// Creates a participant.
    fn create(
        &self,
        email: &str,
        display_name: &str,
    ) -> BoxFuture<'static, Result<User, StorageError>>;
    /// Applies a patch to the profile and returns the stored participant.
    /// `None` means that the participant does not exist.
    fn update(
        &self,
        id: &str,
        patch: ProfilePatch,
    ) -> BoxFuture<'static, Result<Option<User>, StorageError>>;
    /// Anonymises the personal data of the participant.
    fn anonymise(&self, id: &str) -> BoxFuture<'static, Result<(), StorageError>>;
}

/// The tokens: the magic links and the refresh tokens.
pub trait TokenRepository: Send + Sync {
    /// Looks a token up by the hash of its plaintext.
    fn find_by_hash(
        &self,
        token_hash: &str,
    ) -> BoxFuture<'static, Result<Option<StoredToken>, StorageError>>;
    /// Stores a magic link.
    ///
    /// The link belongs to nobody yet: the participant is bound to it when
    /// it is consumed.
    ///
    /// Returns `false` when the single pending slot of the address is
    /// taken by another request: the caller retries. The endpoint answers
    /// `202` either way, so a race is not an error the client can see
    /// (`docs/api-contract.md` §3.1).
    fn insert_magic_link(
        &self,
        email: &str,
        token_hash: &str,
        expires_at: i64,
    ) -> BoxFuture<'static, Result<bool, StorageError>>;
    /// Cancels the magic links of an email that are still pending, so that a
    /// new link can be issued for the same address.
    fn revoke_pending_magic_links(
        &self,
        email: &str,
    ) -> BoxFuture<'static, Result<(), StorageError>>;
    /// Consumes a magic link. Returns `false` when it was already consumed:
    /// the check and the write are one atomic statement.
    ///
    /// The participant is bound to the link in a second step
    /// (`bind_token_user`): the account is created only after the link was
    /// won, so a race cannot leave an orphan account behind.
    fn consume_magic_link(&self, id: &str) -> BoxFuture<'static, Result<bool, StorageError>>;
    /// Binds the participant to a link that was already consumed.
    fn bind_token_user(
        &self,
        id: &str,
        user_id: &str,
    ) -> BoxFuture<'static, Result<(), StorageError>>;
    /// Creates the family of a refresh token.
    ///
    /// The family exists before its first token: the foreign key of
    /// `auth_tokens` rejects a token of a family that does not exist, and
    /// the death of the family is what takes its tokens down (`FR-ID-04`).
    fn create_family(&self, user_id: &str) -> BoxFuture<'static, Result<String, StorageError>>;
    /// Stores a refresh token.
    fn insert_refresh(
        &self,
        user_id: &str,
        family_id: &str,
        token_hash: &str,
        expires_at: i64,
    ) -> BoxFuture<'static, Result<(), StorageError>>;
    /// Cancels one token.
    /// Revokes one token.
    ///
    /// Returns `true` when this call is the one that revoked it, and
    /// `false` when the token was already revoked. The distinction is what
    /// lets the service tell a rotation apart from a reuse: two concurrent
    /// refreshes with the same token race, and only the winner may mint a
    /// successor. The loser has to kill the family, because that is what
    /// `FR-ID-04` asks for.
    ///
    /// The owner of the token travels with the call: the policies of the
    /// schema admit the write only with the context of the participant that
    /// owns the token.
    fn revoke_token(
        &self,
        id: &str,
        user_id: &str,
    ) -> BoxFuture<'static, Result<bool, StorageError>>;
    /// Cancels every token of a family.
    ///
    /// The owner of the family travels with the call: the policies of the
    /// schema admit the write only with the context of the participant that
    /// owns the family.
    fn revoke_family(
        &self,
        family_id: &str,
        user_id: &str,
    ) -> BoxFuture<'static, Result<(), StorageError>>;
    /// Cancels every token of a participant, whatever the family.
    fn revoke_all_for_user(&self, user_id: &str) -> BoxFuture<'static, Result<(), StorageError>>;
}

/// The push devices.
pub trait DeviceRepository: Send + Sync {
    /// Registers the device of the participant, or opens the verification of
    /// the takeover when the token belongs to somebody else.
    ///
    /// The code of the verification is generated by the worker and travels
    /// as its hash: the plaintext is in the SMS and nowhere else.
    fn request_takeover(
        &self,
        user_id: &str,
        platform: &str,
        token: &str,
        app_version: Option<&str>,
        code_hash: &str,
        expires_at: i64,
    ) -> BoxFuture<'static, Result<DeviceRequest, StorageError>>;
    /// The phone of the participant that currently owns the token, so that
    /// the SMS can be addressed to it.
    ///
    /// `None` when that participant has no phone on file: a takeover whose
    /// owner cannot be reached cannot be verified.
    fn takeover_phone(
        &self,
        verification_id: &str,
        user_id: &str,
    ) -> BoxFuture<'static, Result<Option<String>, StorageError>>;
    /// The verification of the participant, with the attempts that were made.
    fn verification(
        &self,
        verification_id: &str,
        user_id: &str,
    ) -> BoxFuture<'static, Result<Option<DeviceVerification>, StorageError>>;
    /// Confirms the takeover with the code.
    ///
    /// `None` when the code is wrong: the attempt is counted by the
    /// database, and the transaction is committed (the refusal is a result,
    /// not an error of the transaction).
    fn confirm_takeover(
        &self,
        verification_id: &str,
        user_id: &str,
        code: &str,
    ) -> BoxFuture<'static, Result<Option<Device>, StorageError>>;
    /// Releases the devices of the participant.
    ///
    /// A device belongs to a session, not to a person: the logout hands the
    /// token back so that the same phone can open another account.
    fn release(&self, user_id: &str) -> BoxFuture<'static, Result<(), StorageError>>;
    /// Deactivates a device of the participant. Returns `false` when the
    /// device does not belong to the participant.
    fn deactivate(
        &self,
        user_id: &str,
        device_id: &str,
    ) -> BoxFuture<'static, Result<bool, StorageError>>;
}

/// The consents, per business and per scope.
pub trait ConsentRepository: Send + Sync {
    /// The current state of every consent of the participant.
    fn list(&self, user_id: &str) -> BoxFuture<'static, Result<Vec<ConsentState>, StorageError>>;
    /// Records a new state for one scope. The table is append only: the last
    /// row of a scope is the one that counts.
    ///
    /// The agent of the request travels with the record: the consent has to
    /// be auditable (`FR-PR-02`).
    fn record(
        &self,
        user_id: &str,
        business_id: &str,
        scope: ConsentScope,
        granted: bool,
        user_agent: Option<&str>,
    ) -> BoxFuture<'static, Result<(), StorageError>>;
}
