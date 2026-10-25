//! The rate limits of the API.
//!
//! The seven groups live in `docs/api-contract.md` §1.6. This module is
//! the pure part of them: what is counted, under which key, and what the
//! answer is when the limit is reached. The store of the counters is
//! behind the `RateLimiter` port (`crate::ports`), and its Redis
//! implementation is in `crate::adapters`.
//!
//! The rule of the layering (`AGENTS.md` §5.1) holds here: nothing in this
//! module imports `iii_sdk` or `sqlx`, and nothing in it reads the clock
//! or the network.

pub mod client;
mod group;
mod subject;

use std::time::Duration;

use uuid::Uuid;

pub use client::client_ip;
pub use group::{RateGroup, RateScope};
pub use subject::{LimitKey, RateSubject};

use crate::ports::{RateLimitError, RateLimiter};

/// The state of a counter after one hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitUsage {
    /// How many hits the window has, this one included.
    pub count: u32,
    /// How long until the window resets.
    pub retry_after: Duration,
}

/// What the enforcement decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateDecision {
    /// The request passes.
    Allowed,
    /// The request is refused.
    ///
    /// It carries the numbers of the answer of the contract: the seconds
    /// to wait and the hits that are left.
    Limited {
        retry_after: Duration,
        remaining: u32,
    },
}

/// The subjects a request carries.
///
/// A dimension that is not here does not apply: there is no address
/// without an edge in front, and the group still counts the dimensions
/// that the request does carry.
#[derive(Debug, Clone, Default)]
pub struct SubjectSet {
    ip: Option<String>,
    email: Option<String>,
    user: Option<Uuid>,
    token: Option<String>,
    business: Option<Uuid>,
}

impl SubjectSet {
    /// An empty set of subjects.
    pub fn new() -> Self {
        Self::default()
    }

    /// The address of the client.
    pub fn with_ip(mut self, ip: Option<String>) -> Self {
        self.ip = ip;
        self
    }

    /// The address of the participant.
    pub fn with_email(mut self, email: Option<String>) -> Self {
        self.email = email;
        self
    }

    /// The participant.
    pub fn with_user(mut self, user: Option<Uuid>) -> Self {
        self.user = user;
        self
    }

    /// The access token.
    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token;
        self
    }

    /// The commerce.
    pub fn with_business(mut self, business: Option<Uuid>) -> Self {
        self.business = business;
        self
    }

    /// The subject of a dimension, if the request carries it.
    fn subject(&self, scope: RateScope) -> Option<RateSubject> {
        match scope {
            RateScope::Ip => self.ip.as_deref().map(RateSubject::ip),
            RateScope::Email => self.email.as_deref().map(RateSubject::email),
            RateScope::User => self.user.map(RateSubject::User),
            RateScope::Token => self.token.as_deref().map(RateSubject::token),
            RateScope::Business => self.business.map(RateSubject::Business),
        }
    }
}

/// Applies a group to the subjects of a request.
///
/// The dimensions are counted in the order of the group, and the first one
/// that refuses ends the count: a request that arrives from an exhausted
/// address is not counted against the address of the participant, because
/// it never got that far.
///
/// **A store that is down is not a permission.** The error goes to the
/// caller, and the caller has to refuse the request: a rate limit that
/// lets everything through when it cannot be consulted is not a rate
/// limit.
pub async fn enforce(
    limiter: &dyn RateLimiter,
    group: RateGroup,
    subjects: &SubjectSet,
) -> Result<RateDecision, RateLimitError> {
    for scope in group.scopes() {
        let Some(subject) = subjects.subject(*scope) else {
            continue;
        };
        let key = LimitKey::new(group, &subject);
        let usage = limiter.consume(key, group.window()).await?;

        if usage.count > group.limit() {
            return Ok(RateDecision::Limited {
                retry_after: usage.retry_after,
                remaining: group.limit().saturating_sub(usage.count),
            });
        }
    }

    Ok(RateDecision::Allowed)
}
