//! The rules of a session: the magic link and the rotating refresh token.
//!
//! Everything here is a pure function of the state it receives. The clock
//! comes from the caller, so the expiry and the rotation rules are testable
//! without a database and without a real clock (AGENTS.md §8).

use crate::domain::errors::IdentityError;
use crate::domain::types::StoredToken;

/// How long a magic link is valid: fifteen minutes (`FR-ID-01`).
pub const MAGIC_LINK_TTL: i64 = 900;

/// How long an access token is valid: fifteen minutes (`FR-ID-04`).
pub const ACCESS_TOKEN_TTL: i64 = 900;

/// How long a refresh token is valid: thirty days (`FR-ID-04`).
pub const REFRESH_TOKEN_TTL: i64 = 30 * 24 * 60 * 60;

/// What to do with a magic link the client presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MagicLinkDecision {
    /// The link is alive and can be consumed.
    Consume,
    /// The link was already used (`AUTH_LINK_CONSUMED`).
    Consumed,
    /// The link is past its deadline (`VALIDATION_ERROR`).
    Expired,
    /// The link was cancelled, usually because a newer one was issued.
    Revoked,
    /// The link does not exist.
    Unknown,
}

/// Decides what to do with a magic link.
///
/// An unknown link and an expired one are the same failure for the client:
/// both answer `VALIDATION_ERROR`, so the answer does not tell whether the
/// address ever existed.
pub fn decide_magic_link(token: &StoredToken, now: i64) -> MagicLinkDecision {
    if token.consumed {
        return MagicLinkDecision::Consumed;
    }
    if token.revoked {
        return MagicLinkDecision::Revoked;
    }
    if token.expires_at <= now {
        return MagicLinkDecision::Expired;
    }
    MagicLinkDecision::Consume
}

/// What to do with a refresh token the client presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshDecision {
    /// The token is alive: rotate it into a new one of the same family.
    Rotate,
    /// The token was already rotated: the whole family is compromised and
    /// dies (`FR-ID-04`).
    Reuse,
    /// The token is past its deadline.
    Expired,
    /// The token does not exist.
    Unknown,
}

/// Decides what to do with a refresh token.
///
/// A rotated token keeps its row with the cancelled state, so a token that
/// comes back is a stolen token: the whole family is cancelled and the
/// participant has to authenticate again.
pub fn decide_refresh(token: &StoredToken, now: i64) -> RefreshDecision {
    if token.revoked {
        return RefreshDecision::Reuse;
    }
    if token.expires_at <= now {
        return RefreshDecision::Expired;
    }
    RefreshDecision::Rotate
}

/// The normalised form of an email address.
///
/// The database stores the address in `citext`, so the comparison is case
/// insensitive anyway. Normalising keeps the tokens and the logs consistent.
pub fn normalise_email(raw: &str) -> Result<String, IdentityError> {
    let email = raw.trim().to_lowercase();
    if email.is_empty() {
        return Err(IdentityError::invalid(
            "email",
            "El email no puede estar vacío.",
        ));
    }
    let mut parts = email.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    let extra_at = parts.next().is_some();
    let valid = !extra_at
        && !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.');
    if !valid {
        return Err(IdentityError::invalid(
            "email",
            "El email no tiene un formato válido.",
        ));
    }
    Ok(email)
}

/// The display name of a participant that did not give one.
///
/// The local part of the address is a better default than an empty name: the
/// participant sees it in the app and can change it later.
pub fn default_display_name(email: &str) -> String {
    let local = email.split('@').next().unwrap_or_default();
    let cleaned: String = local
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<String> = cleaned
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect();
    if words.is_empty() {
        "Participante".to_string()
    } else {
        words.join(" ")
    }
}

/// The platforms a push device can have.
pub const PLATFORMS: [&str; 3] = ["android", "ios", "web"];

/// Whether the platform is one the platform accepts.
pub fn is_valid_platform(platform: &str) -> bool {
    PLATFORMS.contains(&platform)
}
