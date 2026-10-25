//! The subject of a counter: who is being counted, and under which key.

use std::net::IpAddr;

use uuid::Uuid;

use super::group::{RateGroup, RateScope};

/// Who a counter counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateSubject {
    /// The address of the client.
    Ip(String),
    /// The address of the participant.
    Email(String),
    /// The participant.
    User(Uuid),
    /// The access token.
    Token(String),
    /// The commerce.
    Business(Uuid),
}

impl RateSubject {
    /// The address of the client.
    ///
    /// The address is written in one way, so that the same client does not
    /// end up with two counters: an IPv6 written in its long form and in
    /// its short form is the same address.
    pub fn ip(raw: &str) -> Self {
        let trimmed = raw.trim();
        let canonical = trimmed
            .parse::<IpAddr>()
            .map(|address| address.to_string())
            .unwrap_or_else(|_| trimmed.to_string());
        RateSubject::Ip(canonical)
    }

    /// The address of the participant.
    ///
    /// The normalisation is the one of the login (`normalise_email` in the
    /// identity): trim and lower case. If the key used another one, the
    /// limit of an address could be eluded by writing it in another way.
    pub fn email(raw: &str) -> Self {
        RateSubject::Email(raw.trim().to_lowercase())
    }

    /// The access token.
    pub fn token(raw: &str) -> Self {
        RateSubject::Token(raw.trim().to_string())
    }

    /// The dimension the subject belongs to.
    pub fn scope(&self) -> RateScope {
        match self {
            RateSubject::Ip(_) => RateScope::Ip,
            RateSubject::Email(_) => RateScope::Email,
            RateSubject::User(_) => RateScope::User,
            RateSubject::Token(_) => RateScope::Token,
            RateSubject::Business(_) => RateScope::Business,
        }
    }

    /// The value of the subject, before it is hashed.
    fn value(&self) -> String {
        match self {
            RateSubject::Ip(value) => value.clone(),
            RateSubject::Email(value) => value.clone(),
            RateSubject::User(id) => id.to_string(),
            RateSubject::Token(value) => value.clone(),
            RateSubject::Business(id) => id.to_string(),
        }
    }
}

/// The key of a counter in the store.
///
/// The subject is **hashed**: the keys live in Redis, and the address of a
/// participant is not something that should be readable by whoever can read
/// the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitKey {
    group: RateGroup,
    scope: RateScope,
    digest: String,
}

impl LimitKey {
    /// The key of the counter of a subject in a group.
    pub fn new(group: RateGroup, subject: &RateSubject) -> Self {
        use sha2::Digest;

        let digest = hex::encode(sha2::Sha256::digest(subject.value().as_bytes()));
        Self {
            group,
            scope: subject.scope(),
            digest,
        }
    }

    /// The key as it goes to the store.
    pub fn as_string(&self) -> String {
        format!(
            "rl:{}:{}:{}",
            self.group.as_str(),
            self.scope.as_str(),
            self.digest
        )
    }
}
