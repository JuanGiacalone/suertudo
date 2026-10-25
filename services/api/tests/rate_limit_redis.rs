#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The adapter of Redis, tested against a **real** Redis.
//!
//! A mocked store would prove that the adapter calls the mock. The things
//! that break in a rate limiter are the ones a mock cannot have: the TTL
//! that does not get set, the window that does not reset, the counter that
//! is incremented twice. So the test needs a real server.
//!
//! The server is at `REDIS_URL`, and it is **not** optional: if it is not
//! there, the test fails. A test that skips itself is a test that does not
//! exist.

use std::time::Duration;

use suertudo_api::adapters::redis::RedisRateLimiter;
use suertudo_api::ports::{RateLimitError, RateLimiter};
use suertudo_api::rate_limit::{LimitKey, RateGroup, RateSubject};

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_string())
}

/// An address that belongs to nobody, so that the counters of one test do
/// not interfere with the counters of another.
fn unique_email() -> String {
    format!("rate-limit-test-{}@example.com", uuid::Uuid::new_v4())
}

async fn limiter() -> RedisRateLimiter {
    RedisRateLimiter::connect(&redis_url())
        .await
        .expect("the Redis of the tests has to be there")
}

#[tokio::test]
async fn the_counter_counts_the_hits() {
    let limiter = limiter().await;
    let subject = RateSubject::email(&unique_email());
    let key = LimitKey::new(RateGroup::Auth, &subject);

    for expected in 1..=3u32 {
        let usage = limiter
            .consume(key.clone(), Duration::from_secs(300))
            .await
            .expect("the store answered");

        assert_eq!(usage.count, expected);
    }
}

#[tokio::test]
async fn the_window_resets_itself() {
    let limiter = limiter().await;
    let subject = RateSubject::email(&unique_email());
    let key = LimitKey::new(RateGroup::Auth, &subject);

    let first = limiter
        .consume(key.clone(), Duration::from_secs(1))
        .await
        .expect("the store answered");
    assert_eq!(first.count, 1);

    tokio::time::sleep(Duration::from_millis(1_200)).await;

    let second = limiter
        .consume(key, Duration::from_secs(1))
        .await
        .expect("the store answered");
    assert_eq!(second.count, 1, "the window had to reset");
}

#[tokio::test]
async fn the_counter_always_has_a_ttl() {
    let limiter = limiter().await;
    let subject = RateSubject::email(&unique_email());
    let key = LimitKey::new(RateGroup::Auth, &subject);

    let usage = limiter
        .consume(key, Duration::from_secs(300))
        .await
        .expect("the store answered");

    // A counter without a TTL is a counter for ever: the limit would be
    // reached once and never again.
    assert!(
        usage.retry_after > Duration::ZERO,
        "the counter has to have a time to live"
    );
    assert!(
        usage.retry_after <= Duration::from_secs(300),
        "the time to live cannot be longer than the window"
    );
}

#[tokio::test]
async fn the_time_to_wait_is_never_zero() {
    let limiter = limiter().await;
    let subject = RateSubject::email(&unique_email());
    let key = LimitKey::new(RateGroup::Auth, &subject);

    let usage = limiter
        .consume(key, Duration::from_secs(300))
        .await
        .expect("the store answered");

    // A `Retry-After: 0` tells the client "try again now", and produces
    // exactly the burst the limit was there to stop.
    assert!(usage.retry_after >= Duration::from_secs(1));
}

#[tokio::test]
async fn two_addresses_have_two_counters() {
    let limiter = limiter().await;
    let one = LimitKey::new(RateGroup::Auth, &RateSubject::email(&unique_email()));
    let two = LimitKey::new(RateGroup::Auth, &RateSubject::email(&unique_email()));

    limiter
        .consume(one.clone(), Duration::from_secs(300))
        .await
        .expect("the store answered");
    let usage = limiter
        .consume(two, Duration::from_secs(300))
        .await
        .expect("the store answered");

    assert_eq!(usage.count, 1, "the counters of two addresses are two");
}

#[tokio::test]
async fn a_worker_without_the_store_does_not_start() {
    // Nothing listens on this port. A worker that cannot reach the store
    // does not serve traffic: a limit that cannot be consulted is not a
    // limit, and a worker that serves traffic without it is the open door
    // the limit was there to close.
    let result = RedisRateLimiter::connect("redis://127.0.0.1:6399/").await;

    assert_eq!(result.err(), Some(RateLimitError::Unavailable));
}
