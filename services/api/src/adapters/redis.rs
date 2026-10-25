//! The counters of the rate limits, in Redis.
//!
//! The counters live in Redis and **not** in the memory of the process:
//! the workers are several, and a limit that each of them counts on its
//! own is a limit that each of them can be walked around by.

use std::time::Duration;

use redis::Script;

use crate::ports::{BoxFuture, RateLimitError, RateLimiter};
use crate::rate_limit::{LimitKey, LimitUsage};

/// The script that counts a hit and gives the time to live of the counter.
///
/// The `INCR` and the `EXPIRE` have to be the **same** operation. If they
/// are two, two requests that arrive at the same time can both find a
/// counter of zero and both leave it without a time to live — and a counter
/// without a time to live is a counter for ever: the limit is reached once
/// and never again.
const COUNT_AND_EXPIRE: &str = r#"
local hits = redis.call('INCR', KEYS[1])
if hits == 1 then
  redis.call('EXPIRE', KEYS[1], ARGV[1])
end
local ttl = redis.call('TTL', KEYS[1])
return {hits, ttl}
"#;

/// The store of the counters, in Redis.
#[derive(Clone)]
pub struct RedisRateLimiter {
    connection: redis::aio::ConnectionManager,
}

impl std::fmt::Debug for RedisRateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The connection is not printed: it can hold the credentials.
        f.debug_struct("RedisRateLimiter").finish_non_exhaustive()
    }
}

impl RedisRateLimiter {
    /// Connects to the store.
    ///
    /// **A worker that cannot reach the store does not start.** A rate
    /// limit that cannot be consulted is not a rate limit, and a worker
    /// that serves traffic without it is the open door the limit was
    /// there to close. The orchestrator restarts the worker, and by then
    /// the store is usually there.
    ///
    /// The retries are few and short on purpose: waiting ten seconds at
    /// the startup is slower than letting the orchestrator try again.
    pub async fn connect(url: &str) -> Result<Self, RateLimitError> {
        let client =
            redis::Client::open(url).map_err(|error| RateLimitError::Invalid(error.to_string()))?;
        let config = redis::aio::ConnectionManagerConfig::new()
            .set_number_of_retries(3)
            .set_min_delay(Duration::from_millis(100))
            .set_max_delay(Duration::from_millis(500));
        let connection = redis::aio::ConnectionManager::new_with_config(client, config)
            .await
            .map_err(|_| RateLimitError::Unavailable)?;
        Ok(Self { connection })
    }
}

impl RateLimiter for RedisRateLimiter {
    fn consume(
        &self,
        key: LimitKey,
        window: Duration,
    ) -> BoxFuture<'static, Result<LimitUsage, RateLimitError>> {
        let mut connection = self.connection.clone();
        let name = key.as_string();
        // A window of less than a second would expire the counter before
        // anybody could read it.
        let seconds = window.as_secs().max(1);

        Box::pin(async move {
            let script = Script::new(COUNT_AND_EXPIRE);
            let (hits, ttl): (i64, i64) = script
                .key(name)
                .arg(seconds)
                .invoke_async(&mut connection)
                .await
                .map_err(|_| RateLimitError::Unavailable)?;

            let count = u32::try_from(hits).unwrap_or(u32::MAX);
            // A time to live of zero would tell the client "try again now",
            // and would produce exactly the burst the limit is there to
            // stop.
            let retry_after = Duration::from_secs(u64::try_from(ttl).unwrap_or(1).max(1));

            Ok(LimitUsage { count, retry_after })
        })
    }
}
