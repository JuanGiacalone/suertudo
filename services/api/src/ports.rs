//! What the domain needs from the outside world.
//!
//! Every trait here is implemented in `crate::adapters`. The domain only
//! ever sees these traits, never the concrete implementations.
//!
//! The traits return a boxed future with a `'static` lifetime on purpose:
//! an implementation owns what it needs instead of borrowing the caller.
//! That keeps the objects usable as `Arc<dyn Trait>` without fighting the
//! borrow checker at every call site.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use thiserror::Error;

use crate::rate_limit::{LimitKey, LimitUsage};

/// A future that can be stored in a trait object.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The store of the counters of the rate limits.
///
/// The counters live in Redis and **not** in the memory of the process:
/// the workers are several, and a limit that each of them counts on its
/// own is a limit that each of them can be walked around by.
pub trait RateLimiter: Send + Sync {
    /// Counts one hit in the counter of a key, and says what the window
    /// looks like afterwards.
    ///
    /// The store **does not know the policy**: it counts and says how long
    /// the window has left. What the count means is decided by the domain,
    /// because the limit belongs to the contract and the contract belongs
    /// to the domain.
    fn consume(
        &self,
        key: LimitKey,
        window: Duration,
    ) -> BoxFuture<'static, Result<LimitUsage, RateLimitError>>;
}

/// The failures of the store.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RateLimitError {
    /// The store is not reachable.
    ///
    /// The request has to be refused: a rate limit that lets everything
    /// through when it cannot be consulted is not a rate limit.
    #[error("the store of the rate limits is not reachable")]
    Unavailable,
    /// The store answered something that makes no sense.
    #[error("the store of the rate limits answered nonsense: {0}")]
    Invalid(String),
}
