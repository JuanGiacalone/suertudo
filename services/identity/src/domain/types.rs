//! The data the identity domain works with.
//!
//! The types here are plain data: no infrastructure types, no serialization
//! derive. The adapters translate them to and from the database, the wire and
//! the JWT, so the domain stays testable without any of them
//! (AGENTS.md §5.1).

/// The role carried by a session.
///
/// The role decides what the caller may do; it never decides *who* the caller
/// is. The identity comes from the session and is injected by the middleware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Participant,
    BusinessOwner,
    PlatformAdmin,
}

impl Role {
    /// The value stored in `users.role` and in the `role` claim of the token.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Participant => "participant",
            Role::BusinessOwner => "business_owner",
            Role::PlatformAdmin => "platform_admin",
        }
    }

    /// Parses the value stored in the database.
    pub fn parse(value: &str) -> Option<Role> {
        match value {
            "participant" => Some(Role::Participant),
            "business_owner" => Some(Role::BusinessOwner),
            "platform_admin" => Some(Role::PlatformAdmin),
            _ => None,
        }
    }
}

/// The lifecycle of an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountStatus {
    Active,
    PendingReview,
    Suspended,
    Deleted,
}

impl AccountStatus {
    /// The value stored in `users.status`.
    pub fn as_str(self) -> &'static str {
        match self {
            AccountStatus::Active => "active",
            AccountStatus::PendingReview => "pending_review",
            AccountStatus::Suspended => "suspended",
            AccountStatus::Deleted => "deleted",
        }
    }

    /// Parses the value stored in the database.
    pub fn parse(value: &str) -> Option<AccountStatus> {
        match value {
            "active" => Some(AccountStatus::Active),
            "pending_review" => Some(AccountStatus::PendingReview),
            "suspended" => Some(AccountStatus::Suspended),
            "deleted" => Some(AccountStatus::Deleted),
            _ => None,
        }
    }
}

/// A participant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// The identifier of the user, as a UUID in its textual form.
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub role: Role,
    pub status: AccountStatus,
    pub phone_e164: Option<String>,
    pub locale: String,
    pub timezone: String,
    /// The global marketing opt-in. The per-business consent lives in
    /// `consent_records` and is never collapsed into this flag.
    pub marketing_opt_in: bool,
    /// The channels the participant wants to be reached through.
    pub notification_preferences: NotificationPreferences,
}

/// The channels the participant wants to be reached through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationPreferences {
    pub push: bool,
    pub email: bool,
    pub marketing: bool,
}

/// The changes a participant asks for on its profile.
///
/// Every field is optional: an absent field is left untouched. The nested
/// `Option` of `phone_e164` distinguishes "do not touch it" from "set it to
/// null", which the contract treats as two different things.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfilePatch {
    pub display_name: Option<String>,
    pub phone_e164: Option<Option<String>>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub notification_preferences: Option<NotificationPreferences>,
}

/// A registered push device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub platform: String,
    pub token: String,
    pub app_version: Option<String>,
    pub active: bool,
}

/// The scopes a participant can grant or revoke per business.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentScope {
    ShareMarketing,
    ShareContact,
}

impl ConsentScope {
    /// The value stored in `consent_records.scope`.
    pub fn as_str(self) -> &'static str {
        match self {
            ConsentScope::ShareMarketing => "share_marketing",
            ConsentScope::ShareContact => "share_contact",
        }
    }
}

/// The current state of one consent scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentState {
    pub business_id: String,
    pub scope: ConsentScope,
    pub granted: bool,
    /// When the state was decided, in seconds since the Unix epoch.
    pub updated_at: i64,
}

/// The kind of token stored in `auth_tokens`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    MagicLink,
    Refresh,
}

impl TokenKind {
    /// The value stored in `auth_tokens.kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            TokenKind::MagicLink => "magic_link",
            TokenKind::Refresh => "refresh",
        }
    }
}

/// A token as the domain sees it: never the plaintext, always the state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredToken {
    pub id: String,
    /// `None` for a magic link issued before the user existed.
    pub user_id: Option<String>,
    pub email: String,
    pub kind: TokenKind,
    /// The family a refresh token belongs to. A rotation keeps the family.
    pub family_id: Option<String>,
    /// When the token stops being valid, in seconds since the Unix epoch.
    pub expires_at: i64,
    /// The token was already used.
    pub consumed: bool,
    /// The token was cancelled, either by a rotation or by a revocation.
    pub revoked: bool,
}

/// The claims of an access token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessClaims {
    /// The user the token belongs to.
    pub subject: String,
    pub role: String,
    /// Issued at, in seconds since the Unix epoch.
    pub issued_at: i64,
    /// Expires at, in seconds since the Unix epoch.
    pub expires_at: i64,
    /// The unique identifier of the token.
    pub token_id: String,
}

/// The claims of a refresh token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshClaims {
    pub subject: String,
    /// The family the token belongs to. Every rotation keeps the family, so a
    /// stolen token can take the whole family down.
    pub family_id: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub token_id: String,
}

/// The claims of a magic link.
///
/// The link is signed so that it can carry the data the participant gave when
/// it asked for it. The state of the link (used, cancelled) still lives in
/// the database: the signature is not what makes it single use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagicLinkClaims {
    pub email: String,
    /// The display name the participant asked for, when it gave one.
    pub name: Option<String>,
    pub issued_at: i64,
    pub expires_at: i64,
    pub token_id: String,
}

/// A session handed to the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    /// The lifetime of the access token, in seconds.
    pub expires_in: i64,
    pub user: User,
}

/// The outcome of asking for a magic link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagicLinkIssued {
    /// The plaintext token. It reaches the client only in development.
    pub token: String,
    /// The lifetime of the link, in seconds.
    pub expires_in: i64,
}
