//! The groups of rate limit.
//!
//! The seven groups live in `docs/api-contract.md` §1.6, and this module
//! is the **only** place where their numbers are written down. A limit
//! that lives in two files is a limit that is different in two files.

use std::time::Duration;

/// What a group counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateScope {
    /// The address of the client, as the edge that fronts the API saw it.
    Ip,
    /// The address of the participant.
    Email,
    /// The participant.
    User,
    /// The access token.
    Token,
    /// The commerce.
    Business,
}

impl RateScope {
    /// The name of the scope, as it appears in the key of the counter.
    pub fn as_str(self) -> &'static str {
        match self {
            RateScope::Ip => "ip",
            RateScope::Email => "email",
            RateScope::User => "user",
            RateScope::Token => "token",
            RateScope::Business => "business",
        }
    }
}

/// The seven groups of `docs/api-contract.md` §1.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateGroup {
    /// The requests of authentication, by address and by participant.
    Auth,
    /// The request of a magic link, by participant.
    AuthLink,
    /// The reading of a raffle, by token.
    RafflesRead,
    /// The reservation of a number, by participant and by address.
    TicketsCreate,
    /// The generation of a theme, by commerce.
    AiGenerate,
    /// The export of data, by commerce.
    Export,
    /// The public content, by address.
    Public,
}

impl RateGroup {
    /// The name of the group, as it appears in the contract.
    pub fn as_str(self) -> &'static str {
        match self {
            RateGroup::Auth => "auth",
            RateGroup::AuthLink => "auth:link",
            RateGroup::RafflesRead => "raffles:read",
            RateGroup::TicketsCreate => "tickets:create",
            RateGroup::AiGenerate => "ai:generate",
            RateGroup::Export => "export",
            RateGroup::Public => "public",
        }
    }

    /// How many hits the group allows in its window.
    pub fn limit(self) -> u32 {
        match self {
            RateGroup::Auth => 10,
            RateGroup::AuthLink => 5,
            RateGroup::RafflesRead => 120,
            RateGroup::TicketsCreate => 10,
            RateGroup::AiGenerate => 20,
            RateGroup::Export => 5,
            RateGroup::Public => 300,
        }
    }

    /// How long the window of the group lasts.
    pub fn window(self) -> Duration {
        match self {
            RateGroup::Auth => Duration::from_secs(5 * 60),
            RateGroup::AuthLink => Duration::from_secs(15 * 60),
            RateGroup::RafflesRead => Duration::from_secs(60),
            RateGroup::TicketsCreate => Duration::from_secs(5 * 60),
            RateGroup::AiGenerate => Duration::from_secs(60 * 60),
            RateGroup::Export => Duration::from_secs(60 * 60),
            RateGroup::Public => Duration::from_secs(60),
        }
    }

    /// The dimensions the group counts.
    ///
    /// Every one of them has to pass. A dimension that the request does
    /// not carry —there is no address without an edge in front— simply
    /// does not apply, and the rest of the group still does.
    pub fn scopes(self) -> &'static [RateScope] {
        match self {
            RateGroup::Auth => &[RateScope::Ip, RateScope::Email],
            RateGroup::AuthLink => &[RateScope::Email],
            RateGroup::RafflesRead => &[RateScope::Token],
            RateGroup::TicketsCreate => &[RateScope::User, RateScope::Ip],
            RateGroup::AiGenerate => &[RateScope::Business],
            RateGroup::Export => &[RateScope::Business],
            RateGroup::Public => &[RateScope::Ip],
        }
    }

    /// The minimum separation between two hits, for the groups that keep a
    /// durable record of the last one.
    ///
    /// The group of the magic link allows five links in fifteen minutes.
    /// A counter alone would allow the five of them in the same second,
    /// which is a mail bomb and not five links. The separation is
    /// `window / limit`, and it is **derived**, not written down: a
    /// different limit in the contract would move it by itself.
    pub fn min_spacing(self) -> Option<Duration> {
        match self {
            RateGroup::AuthLink => Some(self.window() / self.limit()),
            _ => None,
        }
    }
}
