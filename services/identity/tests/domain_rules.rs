// A failing assertion should stop the run with a readable message: the lint
// exists to keep the panics out of the production paths, not out of the
// tests.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The rules of the identity, without a database.
//!
//! The clock and the randomness come from the ports, so the expiry and the
//! rotation are decided here and not by waiting (AGENTS.md §8).

use suertudo_identity::adapters::Argon2PasswordHasher;
use suertudo_identity::config::{Config, ConfigError, Environment, SmsProvider};
use suertudo_identity::domain::profile::{is_valid_phone, validate_device, validate_patch};
use suertudo_identity::domain::session::{
    MagicLinkDecision, RefreshDecision, decide_magic_link, decide_refresh, default_display_name,
    normalise_email,
};
use suertudo_identity::domain::types::{ProfilePatch, StoredToken, TokenKind};
use suertudo_identity::ports::PasswordHasher;

mod common;
use common::{ACCESS_SECRET, REFRESH_SECRET};

const NOW: i64 = 1_800_000_000;

fn stored_token(kind: TokenKind, expires_at: i64) -> StoredToken {
    StoredToken {
        id: "token".to_string(),
        user_id: Some("user".to_string()),
        email: "ana@example.com".to_string(),
        kind,
        family_id: Some("family".to_string()),
        expires_at,
        consumed: false,
        revoked: false,
    }
}

#[test]
fn a_magic_link_is_consumed_only_once() {
    let mut token = stored_token(TokenKind::MagicLink, NOW + 900);

    assert_eq!(decide_magic_link(&token, NOW), MagicLinkDecision::Consume);

    token.consumed = true;
    assert_eq!(decide_magic_link(&token, NOW), MagicLinkDecision::Consumed);
}

#[test]
fn an_expired_magic_link_is_not_consumable() {
    let token = stored_token(TokenKind::MagicLink, NOW);

    assert_eq!(decide_magic_link(&token, NOW), MagicLinkDecision::Expired);
}

#[test]
fn a_rotated_refresh_token_is_a_reuse() {
    let mut token = stored_token(TokenKind::Refresh, NOW + 30 * 24 * 60 * 60);

    assert_eq!(decide_refresh(&token, NOW), RefreshDecision::Rotate);

    // The token comes back: it was rotated and somebody kept it.
    token.revoked = true;
    assert_eq!(decide_refresh(&token, NOW), RefreshDecision::Reuse);
}

#[test]
fn an_expired_refresh_token_is_not_rotated() {
    let token = stored_token(TokenKind::Refresh, NOW);

    assert_eq!(decide_refresh(&token, NOW), RefreshDecision::Expired);
}

#[test]
fn the_email_is_normalised_before_it_is_used() {
    assert_eq!(
        normalise_email("  Ana@Example.COM ").unwrap(),
        "ana@example.com"
    );
    assert!(normalise_email("not-an-email").is_err());
    assert!(normalise_email("").is_err());
}

#[test]
fn the_display_name_falls_back_to_the_address() {
    assert_eq!(default_display_name("ana.lopez@example.com"), "Ana Lopez");
    assert_eq!(default_display_name("42@example.com"), "42");
}

#[test]
fn the_profile_patch_reports_every_field_that_failed() {
    let patch = ProfilePatch {
        display_name: Some("   ".to_string()),
        phone_e164: Some(Some("1234".to_string())),
        locale: Some(String::new()),
        timezone: None,
        notification_preferences: None,
    };

    let error = validate_patch(&patch).unwrap_err();

    assert_eq!(error.code(), "VALIDATION_ERROR");
    assert_eq!(error.status(), 400);
    assert!(error.fields().contains(&"display_name".to_string()));
    assert!(error.fields().contains(&"phone_e164".to_string()));
    assert!(error.fields().contains(&"locale".to_string()));
}

#[test]
fn a_phone_in_the_e164_form_is_accepted() {
    assert!(is_valid_phone("+5491122334455"));
    assert!(!is_valid_phone("5491122334455"));
    assert!(!is_valid_phone("+54a91122334455"));
}

#[test]
fn a_device_with_an_unknown_platform_is_rejected() {
    let error = validate_device("windows", "token").unwrap_err();

    assert_eq!(error.code(), "VALIDATION_ERROR");
    assert!(error.fields().contains(&"platform".to_string()));
}

#[test]
fn a_password_is_hashed_and_verified() {
    let hasher = Argon2PasswordHasher;

    let hash = hasher.hash("una contraseña").unwrap();
    assert!(!hash.contains("una contraseña"));
    assert!(hasher.verify("una contraseña", &hash).unwrap());
    assert!(!hasher.verify("otra contraseña", &hash).unwrap());
}

/// The variables of a configuration, as a map.
fn vars(entries: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let owned: Vec<(String, String)> = entries
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    move |key: &str| {
        owned
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    }
}

#[test]
fn the_magic_link_travels_in_the_response_only_in_development() {
    let config = Config::from_vars(vars(&[
        ("ENV", "development"),
        ("DATABASE_URL", "postgres://localhost/suertudo"),
        ("JWT_SECRET", ACCESS_SECRET),
        ("JWT_REFRESH_SECRET", REFRESH_SECRET),
        ("REDIS_URL", "redis://127.0.0.1:6379/"),
    ]))
    .unwrap();

    assert_eq!(config.environment, Environment::Development);
    assert!(config.dev_magic_link);
}

/// The worker refuses to start in production with the sender of
/// development.
///
/// The sender of development writes the code of the verification in the log
/// of the worker: nobody deploys a service that writes the codes of its
/// users in the log.
#[test]
fn the_worker_refuses_to_start_in_production_with_a_development_sender() {
    let error = Config::from_vars(vars(&[
        ("ENV", "production"),
        ("DATABASE_URL", "postgres://localhost/suertudo"),
        ("JWT_SECRET", ACCESS_SECRET),
        ("JWT_REFRESH_SECRET", REFRESH_SECRET),
        ("REDIS_URL", "redis://127.0.0.1:6379/"),
    ]))
    .unwrap_err();

    assert_eq!(error, ConfigError::DevSmsInProduction);
}

#[test]
fn the_development_sender_in_staging_needs_the_explicit_consent() {
    // The default of SMS_PROVIDER is the sender of development, so a
    // deployment that forgets the variable must not start writing the
    // codes in the log without having said so.
    let error = Config::from_vars(vars(&[
        ("ENV", "staging"),
        ("DATABASE_URL", "postgres://localhost/suertudo"),
        ("JWT_SECRET", ACCESS_SECRET),
        ("JWT_REFRESH_SECRET", REFRESH_SECRET),
        ("REDIS_URL", "redis://127.0.0.1:6379/"),
    ]))
    .unwrap_err();

    assert_eq!(error, ConfigError::DevSmsNeedsExplicitConsent);
}

#[test]
fn the_development_sender_starts_in_staging_with_the_explicit_consent() {
    let config = Config::from_vars(vars(&[
        ("ENV", "staging"),
        ("DEV_SMS_SENDER", "true"),
        ("DATABASE_URL", "postgres://localhost/suertudo"),
        ("JWT_SECRET", ACCESS_SECRET),
        ("JWT_REFRESH_SECRET", REFRESH_SECRET),
        ("REDIS_URL", "redis://127.0.0.1:6379/"),
    ]))
    .expect("staging may run with the sender of development when it is asked for");

    assert_eq!(config.sms_provider, SmsProvider::Logging);
    assert_eq!(config.environment, Environment::Staging);
}

#[test]
fn the_explicit_consent_does_not_open_production() {
    // The consent of staging is not a way around production.
    let error = Config::from_vars(vars(&[
        ("ENV", "production"),
        ("DEV_SMS_SENDER", "true"),
        ("DATABASE_URL", "postgres://localhost/suertudo"),
        ("JWT_SECRET", ACCESS_SECRET),
        ("JWT_REFRESH_SECRET", REFRESH_SECRET),
        ("REDIS_URL", "redis://127.0.0.1:6379/"),
    ]))
    .unwrap_err();

    assert_eq!(error, ConfigError::DevSmsInProduction);
}

#[test]
fn the_development_sender_starts_in_development_without_asking_for_it() {
    let config = Config::from_vars(vars(&[
        ("ENV", "development"),
        ("DATABASE_URL", "postgres://localhost/suertudo"),
        ("JWT_SECRET", ACCESS_SECRET),
        ("JWT_REFRESH_SECRET", REFRESH_SECRET),
        ("REDIS_URL", "redis://127.0.0.1:6379/"),
    ]))
    .expect("development may run with the sender of development");

    assert_eq!(config.sms_provider, SmsProvider::Logging);
}

#[test]
fn the_worker_refuses_to_start_with_the_magic_link_outside_development() {
    let error = Config::from_vars(vars(&[
        ("ENV", "production"),
        ("DEV_MAGIC_LINK", "true"),
        ("DATABASE_URL", "postgres://localhost/suertudo"),
        ("JWT_SECRET", ACCESS_SECRET),
        ("JWT_REFRESH_SECRET", REFRESH_SECRET),
        ("REDIS_URL", "redis://127.0.0.1:6379/"),
    ]))
    .unwrap_err();

    assert_eq!(error, ConfigError::DevMagicLinkOutsideDevelopment);
}

#[test]
fn the_two_signing_keys_have_to_be_different() {
    let error = Config::from_vars(vars(&[
        ("ENV", "production"),
        ("DATABASE_URL", "postgres://localhost/suertudo"),
        ("JWT_SECRET", ACCESS_SECRET),
        ("JWT_REFRESH_SECRET", ACCESS_SECRET),
    ]))
    .unwrap_err();

    assert_eq!(error, ConfigError::SameSecret);
}

#[test]
fn a_short_signing_key_is_refused() {
    let error = Config::from_vars(vars(&[
        ("ENV", "production"),
        ("DATABASE_URL", "postgres://localhost/suertudo"),
        ("JWT_SECRET", "corto"),
        ("JWT_REFRESH_SECRET", REFRESH_SECRET),
        ("REDIS_URL", "redis://127.0.0.1:6379/"),
    ]))
    .unwrap_err();

    assert_eq!(error, ConfigError::WeakSecret("JWT_SECRET"));
}
