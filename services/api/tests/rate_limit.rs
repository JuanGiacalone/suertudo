#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The rules of the rate limit, tested through the public surface of the
//! crate.
//!
//! The domain is pure: the groups, the subjects, the key and the decision
//! are decided without a clock, without a store and without a network. The
//! adapter of Redis has its own test, against a real Redis.

use std::collections::HashMap;
use std::time::Duration;

use suertudo_api::ports::{RateLimitError, RateLimiter};
use suertudo_api::rate_limit::{
    LimitKey, RateDecision, RateGroup, RateScope, RateSubject, SubjectSet, client_ip, enforce,
};

// ── The groups of `docs/api-contract.md` §1.6 ──────────────────────────────

#[test]
fn the_seven_groups_have_the_limits_and_the_windows_of_the_contract() {
    let expected = [
        (RateGroup::Auth, 10, 5 * 60),
        (RateGroup::AuthLink, 5, 15 * 60),
        (RateGroup::RafflesRead, 120, 60),
        (RateGroup::TicketsCreate, 10, 5 * 60),
        (RateGroup::AiGenerate, 20, 60 * 60),
        (RateGroup::Export, 5, 60 * 60),
        (RateGroup::Public, 300, 60),
    ];

    for (group, limit, window) in expected {
        assert_eq!(group.limit(), limit, "the limit of {}", group.as_str());
        assert_eq!(
            group.window(),
            Duration::from_secs(window),
            "the window of {}",
            group.as_str()
        );
    }
}

#[test]
fn every_group_counts_the_dimensions_the_contract_says() {
    assert_eq!(RateGroup::Auth.scopes(), &[RateScope::Ip, RateScope::Email]);
    assert_eq!(RateGroup::AuthLink.scopes(), &[RateScope::Email]);
    assert_eq!(RateGroup::RafflesRead.scopes(), &[RateScope::Token]);
    assert_eq!(
        RateGroup::TicketsCreate.scopes(),
        &[RateScope::User, RateScope::Ip]
    );
    assert_eq!(RateGroup::AiGenerate.scopes(), &[RateScope::Business]);
    assert_eq!(RateGroup::Export.scopes(), &[RateScope::Business]);
    assert_eq!(RateGroup::Public.scopes(), &[RateScope::Ip]);
}

#[test]
fn the_name_of_a_group_is_the_one_of_the_contract() {
    assert_eq!(RateGroup::Auth.as_str(), "auth");
    assert_eq!(RateGroup::AuthLink.as_str(), "auth:link");
    assert_eq!(RateGroup::RafflesRead.as_str(), "raffles:read");
    assert_eq!(RateGroup::TicketsCreate.scopes()[0], RateScope::User);
    assert_eq!(RateGroup::AiGenerate.as_str(), "ai:generate");
    assert_eq!(RateGroup::Export.as_str(), "export");
    assert_eq!(RateGroup::Public.as_str(), "public");
}

#[test]
fn the_group_of_the_magic_link_keeps_the_durable_record_apart() {
    // The group allows five links in fifteen minutes, so two links to the
    // same address have to be at least three minutes apart: otherwise the
    // five of them could arrive in the same second.
    assert_eq!(
        RateGroup::AuthLink.min_spacing(),
        Some(Duration::from_secs(3 * 60))
    );
    // The other groups keep no durable record.
    assert_eq!(RateGroup::Auth.min_spacing(), None);
}

// ── The key of a counter ──────────────────────────────────────────────────

#[test]
fn the_key_of_a_counter_does_not_carry_the_address_of_the_participant() {
    let key = LimitKey::new(RateGroup::Auth, &RateSubject::email("ana@example.com"));

    assert!(
        !key.as_string().contains("ana@example.com"),
        "the key of a counter must not carry the address: {}",
        key.as_string()
    );
    assert!(key.as_string().starts_with("rl:auth:email:"));
}

#[test]
fn two_subjects_of_the_same_group_have_two_keys() {
    let a = LimitKey::new(RateGroup::Auth, &RateSubject::email("ana@example.com"));
    let b = LimitKey::new(RateGroup::Auth, &RateSubject::email("beto@example.com"));

    assert_ne!(a, b);
}

#[test]
fn the_same_subject_in_two_groups_has_two_keys() {
    let subject = RateSubject::email("ana@example.com");
    let auth = LimitKey::new(RateGroup::Auth, &subject);
    let link = LimitKey::new(RateGroup::AuthLink, &subject);

    assert_ne!(auth, link);
}

#[test]
fn an_address_is_the_same_address_however_it_is_written() {
    // The login normalises the address before using it, and the key has to
    // use the SAME normalisation: otherwise the limit of an address could
    // be eluded by writing it in another way.
    let one = LimitKey::new(RateGroup::Auth, &RateSubject::email("Ana@Example.com"));
    let two = LimitKey::new(RateGroup::Auth, &RateSubject::email("  ana@example.com  "));

    assert_eq!(one, two);
}

// ── The address of the client ─────────────────────────────────────────────

fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect()
}

#[test]
fn the_address_of_the_client_is_the_last_one_of_the_forwarded_header() {
    // The edge APPENDS the address it saw to the header, so everything
    // before the last entry comes from the client and cannot be trusted.
    let ip = client_ip(&headers(&[("x-forwarded-for", "10.0.0.1, 203.0.113.9")]));

    assert_eq!(ip.as_deref(), Some("203.0.113.9"));
}

#[test]
fn a_client_that_forges_the_forwarded_header_does_not_move_the_address() {
    let ip = client_ip(&headers(&[("x-forwarded-for", "1.2.3.4")]));

    // There is no edge in front: the only value is the one the client
    // sent, and it is the last one. The trust is structural: the engine is
    // not exposed to the internet (ADR-012).
    assert_eq!(ip.as_deref(), Some("1.2.3.4"));
}

#[test]
fn a_request_without_the_forwarded_header_has_no_address() {
    assert_eq!(client_ip(&headers(&[])), None);
    assert_eq!(client_ip(&headers(&[("x-forwarded-for", "")])), None);
}

#[test]
fn the_address_of_the_client_is_written_in_one_way() {
    // The same address written in two ways is the same address: the
    // counter of one client must not be split in two. The extraction is
    // literal —it returns what the edge sent— and the CANONICAL form is
    // the one of the key.
    let one = client_ip(&headers(&[(
        "x-forwarded-for",
        "2001:0DB8:0000:0000:0000:0000:0000:0001",
    )]));
    let two = client_ip(&headers(&[("x-forwarded-for", "2001:db8::1")]));

    let key_one = LimitKey::new(
        RateGroup::Public,
        &RateSubject::ip(one.as_deref().unwrap_or("")),
    );
    let key_two = LimitKey::new(
        RateGroup::Public,
        &RateSubject::ip(two.as_deref().unwrap_or("")),
    );

    assert_eq!(key_one, key_two);
}

// ── The decision ──────────────────────────────────────────────────────────

/// A limiter that always lets the request pass.
struct AllowAll;

impl RateLimiter for AllowAll {
    fn consume(
        &self,
        _key: LimitKey,
        _window: Duration,
    ) -> suertudo_api::ports::BoxFuture<
        'static,
        Result<suertudo_api::rate_limit::LimitUsage, RateLimitError>,
    > {
        Box::pin(async move {
            Ok(suertudo_api::rate_limit::LimitUsage {
                count: 1,
                retry_after: Duration::from_secs(60),
            })
        })
    }
}

/// A limiter that always refuses.
struct DenyAll;

impl RateLimiter for DenyAll {
    fn consume(
        &self,
        _key: LimitKey,
        _window: Duration,
    ) -> suertudo_api::ports::BoxFuture<
        'static,
        Result<suertudo_api::rate_limit::LimitUsage, RateLimitError>,
    > {
        Box::pin(async move {
            Ok(suertudo_api::rate_limit::LimitUsage {
                count: 1_000,
                retry_after: Duration::from_secs(42),
            })
        })
    }
}

#[tokio::test]
async fn a_request_that_passes_every_dimension_is_allowed() {
    let subjects = SubjectSet::new()
        .with_ip(Some("203.0.113.9".to_string()))
        .with_email(Some("ana@example.com".to_string()));

    let decision = enforce(&AllowAll, RateGroup::Auth, &subjects)
        .await
        .expect("the store answered");

    assert_eq!(decision, RateDecision::Allowed);
}

#[tokio::test]
async fn a_request_that_exhausts_a_dimension_is_refused() {
    let subjects = SubjectSet::new()
        .with_ip(Some("203.0.113.9".to_string()))
        .with_email(Some("ana@example.com".to_string()));

    let decision = enforce(&DenyAll, RateGroup::Auth, &subjects)
        .await
        .expect("the store answered");

    match decision {
        RateDecision::Limited {
            retry_after,
            remaining,
        } => {
            assert_eq!(remaining, 0);
            assert_eq!(retry_after, Duration::from_secs(42));
        }
        RateDecision::Allowed => panic!("a refused request was allowed"),
    }
}

#[tokio::test]
async fn a_dimension_the_request_does_not_carry_is_not_counted() {
    // There is no edge in front: the request has no address, and the
    // dimension by IP simply does not apply. The dimension by email still
    // does, and that is the one that closes the vector of abuse.
    let subjects = SubjectSet::new().with_email(Some("ana@example.com".to_string()));

    let decision = enforce(&AllowAll, RateGroup::Auth, &subjects)
        .await
        .expect("the store answered");

    assert_eq!(decision, RateDecision::Allowed);
}

#[tokio::test]
async fn a_store_that_is_not_reachable_is_not_a_permission() {
    struct Broken;

    impl RateLimiter for Broken {
        fn consume(
            &self,
            _key: LimitKey,
            _window: Duration,
        ) -> suertudo_api::ports::BoxFuture<
            'static,
            Result<suertudo_api::rate_limit::LimitUsage, RateLimitError>,
        > {
            Box::pin(async { Err(RateLimitError::Unavailable) })
        }
    }

    let subjects = SubjectSet::new().with_email(Some("ana@example.com".to_string()));

    let result = enforce(&Broken, RateGroup::AuthLink, &subjects).await;

    assert!(
        result.is_err(),
        "a store that is down must NOT be a permission to pass"
    );
}
