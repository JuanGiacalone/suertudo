// A failing assertion should stop the run with a readable message: the lint
// exists to keep the panics out of the production paths, not out of the
// tests.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The endpoints of the identity, against a real database.
//!
//! The rules that matter are the ones of the contract: the magic link works
//! once, the refresh token rotates and takes its family down when it comes
//! back, and the session of the caller is the only source of the identity.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{
    BarrierTokens, TestClock, audit_rows, data, deps, deps_with_tokens, error_code, harness,
    lock_users, request, request_with_token, status, ticket_of, ticket_owner, unique_email,
    waiting_writers,
};
use serde_json::json;
use suertudo_identity::handlers;
use suertudo_identity::middleware::authorize;

const NOW: i64 = 1_800_000_000;

/// Asks for a magic link and returns the token of the answer.
async fn magic_link(
    deps: &Arc<suertudo_identity::Deps>,
    email: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let answer = handlers::request_magic_link(deps, request(json!({ "email": email }))).await;
    assert_eq!(status(&answer), 202);
    let token = data(&answer)["dev_magic_link"]
        .as_str()
        .ok_or("the development mode did not return the magic link")?
        .to_string();
    Ok(token)
}

/// Consumes a magic link and returns the session of the answer.
async fn session(
    deps: &Arc<suertudo_identity::Deps>,
    token: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let answer = handlers::verify_magic_link(deps, request(json!({ "token": token }))).await;
    if status(&answer) != 200 {
        return Err(format!("the magic link was not accepted: {answer}").into());
    }
    Ok(data(&answer).clone())
}

#[tokio::test]
async fn the_magic_link_always_answers_202() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let deps = setup.deps.clone();

    // An address that does not exist.
    let unknown = unique_email();
    let answer = handlers::request_magic_link(&deps, request(json!({ "email": unknown }))).await;
    assert_eq!(status(&answer), 202);
    assert_eq!(data(&answer)["sent"], true);
    assert_eq!(data(&answer)["expires_in"], 900);
    assert_eq!(data(&answer)["delivery"], "email");

    // An address that exists.
    let known = unique_email();
    let token = magic_link(&deps, &known).await?;
    session(&deps, &token).await?;

    let answer = handlers::request_magic_link(&deps, request(json!({ "email": known }))).await;
    assert_eq!(status(&answer), 202);

    Ok(())
}

#[tokio::test]
async fn a_magic_link_works_exactly_once() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let deps = setup.deps.clone();

    let token = magic_link(&deps, &unique_email()).await?;

    let first = handlers::verify_magic_link(&deps, request(json!({ "token": token }))).await;
    assert_eq!(status(&first), 200);

    let second = handlers::verify_magic_link(&deps, request(json!({ "token": token }))).await;
    assert_eq!(status(&second), 400);
    assert_eq!(error_code(&second), "AUTH_LINK_CONSUMED");

    Ok(())
}

#[tokio::test]
async fn an_expired_magic_link_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let deps = setup.deps.clone();

    let token = magic_link(&deps, &unique_email()).await?;

    // The link lives fifteen minutes.
    clock.advance(901);

    let answer = handlers::verify_magic_link(&deps, request(json!({ "token": token }))).await;
    assert_eq!(status(&answer), 400);
    assert_eq!(error_code(&answer), "VALIDATION_ERROR");

    Ok(())
}

#[tokio::test]
async fn the_refresh_token_rotates_and_its_reuse_kills_the_family()
-> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let deps = setup.deps.clone();

    let token = magic_link(&deps, &unique_email()).await?;
    let session = session(&deps, &token).await?;
    let refresh = session["refresh_token"].as_str().unwrap().to_string();

    // The rotation issues a new refresh token.
    let answer =
        handlers::refresh_session(&deps, request(json!({ "refresh_token": refresh }))).await;
    assert_eq!(status(&answer), 200);
    let rotated = data(&answer)["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(rotated, refresh);

    // The rotated token comes back: it was stolen.
    let answer =
        handlers::refresh_session(&deps, request(json!({ "refresh_token": refresh }))).await;
    assert_eq!(status(&answer), 401);
    assert_eq!(error_code(&answer), "AUTH_REQUIRED");

    // The whole family is dead, the legitimate token included.
    let answer =
        handlers::refresh_session(&deps, request(json!({ "refresh_token": rotated }))).await;
    assert_eq!(status(&answer), 401);
    assert_eq!(error_code(&answer), "AUTH_REQUIRED");

    Ok(())
}

#[tokio::test]
async fn the_logout_revokes_the_family() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let deps = setup.deps.clone();

    let token = magic_link(&deps, &unique_email()).await?;
    let session = session(&deps, &token).await?;
    let refresh = session["refresh_token"].as_str().unwrap().to_string();

    let answer = handlers::logout(&deps, request(json!({ "refresh_token": refresh }))).await;
    assert_eq!(status(&answer), 200);

    let answer =
        handlers::refresh_session(&deps, request(json!({ "refresh_token": refresh }))).await;
    assert_eq!(status(&answer), 401);
    assert_eq!(error_code(&answer), "AUTH_REQUIRED");

    Ok(())
}

#[tokio::test]
async fn an_expired_access_token_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let deps = setup.deps.clone();

    let token = magic_link(&deps, &unique_email()).await?;
    let session = session(&deps, &token).await?;
    let access = session["access_token"].as_str().unwrap().to_string();

    // The access token lives fifteen minutes.
    clock.advance(901);

    let answer = handlers::get_profile(&deps, request_with_token(json!({}), &access)).await;
    assert_eq!(status(&answer), 401);
    assert_eq!(error_code(&answer), "AUTH_EXPIRED");

    Ok(())
}

#[tokio::test]
async fn the_magic_link_travels_in_the_response_only_in_development()
-> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;

    let development = deps(&setup.application, clock.clone(), true);
    let answer =
        handlers::request_magic_link(&development, request(json!({ "email": unique_email() })))
            .await;
    assert_eq!(status(&answer), 202);
    assert!(data(&answer)["dev_magic_link"].is_string());

    let production = deps(&setup.application, clock, false);
    let answer =
        handlers::request_magic_link(&production, request(json!({ "email": unique_email() })))
            .await;
    assert_eq!(status(&answer), 202);
    assert!(data(&answer).get("dev_magic_link").is_none());

    Ok(())
}

#[tokio::test]
async fn the_middleware_injects_the_context_of_the_caller() -> Result<(), Box<dyn std::error::Error>>
{
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let deps = setup.deps.clone();

    let token = magic_link(&deps, &unique_email()).await?;
    let session = session(&deps, &token).await?;
    let access = session["access_token"].as_str().unwrap().to_string();
    let user_id = session["user"]["id"].as_str().unwrap().to_string();

    let injected = authorize(
        &deps.resolver,
        json!({ "function_id": "participation::reserve_ticket", "payload": {
            "headers": { "authorization": format!("Bearer {access}") },
            "body": {}
        }}),
    )
    .map_err(|err| format!("the middleware rejected a valid session: {err}"))?;

    assert_eq!(
        injected["__auth_context"]["user_id"].as_str(),
        Some(user_id.as_str())
    );

    Ok(())
}

#[tokio::test]
async fn a_request_without_a_session_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let deps = setup.deps.clone();

    // No token at all.
    let answer = handlers::get_profile(&deps, request(json!({}))).await;
    assert_eq!(status(&answer), 401);
    assert_eq!(error_code(&answer), "AUTH_REQUIRED");

    // A token that is not a token.
    let answer = handlers::get_profile(&deps, request_with_token(json!({}), "not-a-token")).await;
    assert_eq!(status(&answer), 401);
    assert_eq!(error_code(&answer), "AUTH_REQUIRED");

    // The middleware rejects it as well.
    let rejected = authorize(
        &deps.resolver,
        json!({ "request": { "headers": { "authorization": "Bearer not-a-token" } } }),
    )
    .map_err(|err| format!("the middleware answered instead of rejecting: {err}"))?;
    assert_eq!(rejected["action"], "respond");
    assert_eq!(rejected["response"]["status_code"], 401);

    Ok(())
}

#[tokio::test]
async fn the_profile_is_the_one_of_the_caller() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let deps = setup.deps.clone();

    let token = magic_link(&deps, &unique_email()).await?;
    let session = session(&deps, &token).await?;
    let access = session["access_token"].as_str().unwrap().to_string();

    let answer = handlers::get_profile(&deps, request_with_token(json!({}), &access)).await;
    assert_eq!(status(&answer), 200);
    assert_eq!(data(&answer)["id"], session["user"]["id"]);
    assert_eq!(data(&answer)["role"], "participant");

    Ok(())
}

#[tokio::test]
async fn the_profile_can_be_updated() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let deps = setup.deps.clone();

    let token = magic_link(&deps, &unique_email()).await?;
    let session = session(&deps, &token).await?;
    let access = session["access_token"].as_str().unwrap().to_string();

    // The phone is unique in the whole platform: every test uses its own.
    let phone = format!("+5491{:08}", uuid::Uuid::new_v4().as_u128() % 100_000_000);
    let answer = handlers::update_profile(
        &deps,
        request_with_token(
            json!({
                "display_name": "Ana Lopez",
                "phone_e164": phone,
                "locale": "es-AR",
                "timezone": "America/Argentina/Buenos_Aires",
            }),
            &access,
        ),
    )
    .await;
    assert_eq!(status(&answer), 200);
    assert_eq!(data(&answer)["display_name"], "Ana Lopez");
    assert_eq!(data(&answer)["phone_e164"], phone);

    // A phone that is not in the E.164 form is rejected.
    let answer = handlers::update_profile(
        &deps,
        request_with_token(json!({ "phone_e164": "1122334455" }), &access),
    )
    .await;
    assert_eq!(status(&answer), 400);
    assert_eq!(error_code(&answer), "VALIDATION_ERROR");

    Ok(())
}

#[tokio::test]
async fn the_name_of_the_request_is_the_name_of_the_participant()
-> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let deps = setup.deps.clone();

    let email = unique_email();
    let answer = handlers::request_magic_link(
        &deps,
        request(json!({ "email": email, "name": "Ana Lopez" })),
    )
    .await;
    assert_eq!(status(&answer), 202);

    let token = data(&answer)["dev_magic_link"]
        .as_str()
        .unwrap()
        .to_string();
    let session = session(&deps, &token).await?;

    // The participant is created when the link is consumed, not before.
    assert_eq!(session["user"]["display_name"], "Ana Lopez");
    assert_eq!(session["user"]["email"], email);
    assert_eq!(session["user"]["role"], "participant");

    Ok(())
}

#[tokio::test]
async fn a_push_device_is_registered_and_removed() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let deps = setup.deps.clone();

    let token = magic_link(&deps, &unique_email()).await?;
    let session = session(&deps, &token).await?;
    let access = session["access_token"].as_str().unwrap().to_string();

    let device_token = format!("device-{}", uuid::Uuid::new_v4());
    let answer = handlers::register_device(
        &deps,
        request_with_token(
            json!({
                "platform": "android",
                "token": device_token,
                "app_version": "1.2.0",
            }),
            &access,
        ),
    )
    .await;
    assert_eq!(status(&answer), 201);
    let device_id = data(&answer)["id"].as_str().unwrap().to_string();

    let mut payload = request_with_token(json!({}), &access);
    payload["path_params"] = json!({ "device_id": device_id });
    let answer = handlers::delete_device(&deps, payload).await;
    assert_eq!(status(&answer), 200);

    // The device is gone.
    let mut payload = request_with_token(json!({}), &access);
    payload["path_params"] = json!({ "device_id": device_id });
    let answer = handlers::delete_device(&deps, payload).await;
    assert_eq!(status(&answer), 404);

    Ok(())
}

#[tokio::test]
async fn the_consent_of_a_business_is_recorded() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let pool = &setup.owner;
    let deps = setup.deps.clone();

    let business_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO businesses (slug, name, category) VALUES ($1, 'Comercio de prueba', 'other') RETURNING id",
    )
    .bind(format!("biz-{}", uuid::Uuid::new_v4()))
    .fetch_one(pool)
    .await?;
    let business_id = business_id.to_string();

    let token = magic_link(&deps, &unique_email()).await?;
    let session = session(&deps, &token).await?;
    let access = session["access_token"].as_str().unwrap().to_string();

    let mut payload = request_with_token(
        json!({ "share_marketing": true, "share_contact": false }),
        &access,
    );
    payload["headers"]["user-agent"] = json!("suertudo-test/1.0");
    payload["path_params"] = json!({ "business_id": business_id });
    let answer = handlers::set_consent(&deps, payload).await;
    assert_eq!(status(&answer), 200);

    // The consent is auditable: the agent of the request travels with it.
    let user_agent: Option<String> = sqlx::query_scalar(
        "SELECT user_agent FROM consent_records WHERE user_id = $1 AND scope = 'share_marketing'",
    )
    .bind(uuid::Uuid::parse_str(
        session["user"]["id"].as_str().unwrap(),
    )?)
    .fetch_one(pool)
    .await?;
    assert_eq!(user_agent.as_deref(), Some("suertudo-test/1.0"));

    let answer = handlers::list_consents(&deps, request_with_token(json!({}), &access)).await;
    assert_eq!(status(&answer), 200);
    let states = data(&answer).as_array().unwrap();
    assert_eq!(states.len(), 2);
    assert!(
        states
            .iter()
            .any(|state| { state["scope"] == "share_marketing" && state["granted"] == true })
    );
    assert!(
        states
            .iter()
            .any(|state| { state["scope"] == "share_contact" && state["granted"] == false })
    );

    Ok(())
}

#[tokio::test]
async fn the_account_deletion_anonymises_the_participant() -> Result<(), Box<dyn std::error::Error>>
{
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let deps = setup.deps.clone();

    let token = magic_link(&deps, &unique_email()).await?;
    let session = session(&deps, &token).await?;
    let access = session["access_token"].as_str().unwrap().to_string();

    let answer = handlers::delete_profile(&deps, request_with_token(json!({}), &access)).await;
    assert_eq!(status(&answer), 202);

    // The profile is gone and the session died with it.
    let answer = handlers::get_profile(&deps, request_with_token(json!({}), &access)).await;
    assert_eq!(status(&answer), 404);

    // The sessions are revoked: the tokens of the participant are dead.
    let revoked: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT revoked_at FROM auth_tokens WHERE user_id = $1 AND kind = 'refresh'",
    )
    .bind(uuid::Uuid::parse_str(
        session["user"]["id"].as_str().unwrap(),
    )?)
    .fetch_one(&setup.owner)
    .await?;
    assert!(
        revoked.is_some(),
        "the session of the participant survived the deletion"
    );

    // And the family of the session is dead too: a successor cannot be
    // minted for it.
    let family: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT revoked_at FROM token_families WHERE user_id = $1")
            .bind(uuid::Uuid::parse_str(
                session["user"]["id"].as_str().unwrap(),
            )?)
            .fetch_one(&setup.owner)
            .await?;
    assert!(
        family.is_some(),
        "the family of the session survived the deletion"
    );

    Ok(())
}

/// A refresh token that was already rotated is a reuse, and the reuse takes
/// the whole family down.
///
/// This is the race that `FR-ID-04` asks for. Two refreshes with the same
/// token can be in flight at the same time. The one that loses the race
/// finds the token already revoked, and it must not mint a second valid
/// successor: it has to kill the family, the successor of the winner
/// included.
///
/// The race is reproduced deterministically: the winner of the race is
/// simulated by revoking the token and minting its successor out of band,
/// which is exactly the state the loser of the race finds.
#[tokio::test]
async fn a_refresh_of_an_already_rotated_token_kills_the_family()
-> Result<(), Box<dyn std::error::Error>> {
    use suertudo_identity::adapters::Sha256Digest;
    use suertudo_identity::ports::Digest;

    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let pool = &setup.owner;
    let deps = setup.deps.clone();

    let email = unique_email();
    let token = magic_link(&deps, &email).await?;
    let session = session(&deps, &token).await?;
    let refresh = session["refresh_token"].as_str().unwrap().to_string();

    // The state the loser of the race finds: the token is revoked and the
    // winner of the race already minted a successor in the same family.
    let tokens = deps.service.tokens.clone();
    let stored = tokens
        .find_by_hash(&Sha256Digest.sha256_hex(&refresh))
        .await?
        .ok_or("the refresh token is not in the store")?;
    let user_id = stored.user_id.clone().ok_or("the token has no user")?;
    let family_id = stored.family_id.clone().ok_or("the token has no family")?;

    assert!(
        tokens.revoke_token(&stored.id, &user_id).await?,
        "the first revoke wins"
    );
    assert!(
        !tokens.revoke_token(&stored.id, &user_id).await?,
        "the second revoke must report that somebody else won the race"
    );

    // The successor is unique: the tests share one database.
    let successor = format!("successor-of-the-winner-{email}");
    tokens
        .insert_refresh(
            &user_id,
            &family_id,
            &Sha256Digest.sha256_hex(&successor),
            NOW + 2_592_000,
        )
        .await?;

    // The loser of the race presents the old token.
    let answer =
        handlers::refresh_session(&deps, request(json!({ "refresh_token": refresh }))).await;
    assert_eq!(status(&answer), 401);
    assert_eq!(error_code(&answer), "AUTH_REQUIRED");

    // And the family is dead: the successor of the winner included.
    let answer = handlers::refresh_session(
        &deps,
        request(json!({ "refresh_token": successor.clone() })),
    )
    .await;
    assert_eq!(
        status(&answer),
        401,
        "the family survived a reuse: {answer}"
    );

    // The death of the family is in the database, and not only in the token
    // that presented it.
    let revoked: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT revoked_at FROM token_families WHERE id = $1")
            .bind(uuid::Uuid::parse_str(&family_id)?)
            .fetch_one(pool)
            .await?;
    assert!(
        revoked.is_some(),
        "the family is alive: a successor could still be minted"
    );

    Ok(())
}

/// The row of a participant is only visible with its context.
///
/// The policies of the schema fail closed: a statement that runs without
/// the context of the participant reads nothing. The adapter reads the row
/// with the context of its owner, and the test proves that the difference
/// is the context and not the connection.
///
/// The two reads use two different pools on purpose: a pool that already
/// ran a transaction keeps the marker of the context in the session, and a
/// statement that runs outside a transaction would look like the platform.
#[tokio::test]
async fn the_row_of_a_participant_is_only_visible_with_its_context()
-> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;

    let token = magic_link(&setup.deps, &unique_email()).await?;
    let session = session(&setup.deps, &token).await?;
    let user_id = session["user"]["id"].as_str().unwrap().to_string();

    // A statement without the context reads nothing: the policies of the
    // schema fail closed.
    let without_context = common::app_pool(&setup.owner).await?;
    let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE id = $1")
        .bind(uuid::Uuid::parse_str(&user_id)?)
        .fetch_one(&without_context)
        .await?;
    assert_eq!(
        visible, 0,
        "the row of a participant is visible without its context"
    );

    // The adapter reads it with the context of the participant.
    let with_context = common::deps(&common::app_pool(&setup.owner).await?, clock, true);
    let user = with_context
        .service
        .users
        .find_by_id(&user_id)
        .await?
        .ok_or("the participant was not found with its context")?;
    assert_eq!(user.id, user_id);

    Ok(())
}

/// A participant that already exists can ask for another link.
///
/// The login path has no tenant context: the participant is not
/// authenticated yet, and the function of the schema resolves the address
/// for it.
#[tokio::test]
async fn an_existing_participant_can_ask_for_another_link() -> Result<(), Box<dyn std::error::Error>>
{
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let email = unique_email();

    // The first link creates the account.
    let token = magic_link(&setup.deps, &email).await?;
    session(&setup.deps, &token).await?;

    // The second link is verified with a pool that has not run a
    // transaction yet: the login path must not depend on the context of the
    // connection to find the participant.
    let answer =
        handlers::request_magic_link(&setup.deps, request(json!({ "email": email }))).await;
    assert_eq!(status(&answer), 202);
    let token = data(&answer)["dev_magic_link"]
        .as_str()
        .unwrap()
        .to_string();

    let fresh = common::deps(&common::app_pool(&setup.owner).await?, clock, true);
    let answer = handlers::verify_magic_link(&fresh, request(json!({ "token": token }))).await;
    assert_eq!(
        status(&answer),
        200,
        "the participant that already exists could not verify a link: {answer}"
    );
    assert_eq!(data(&answer)["user"]["email"], email);

    Ok(())
}

/// The refusal of the database carries the code of the catalogue.
///
/// The database refuses with a SQLSTATE; the answer carries the code of
/// `docs/api-contract.md` §1.4 and the status that goes with it. A refusal
/// that collapses into a `500` is a refusal the client cannot handle.
#[tokio::test]
async fn the_refusal_of_the_database_carries_the_code_of_the_catalogue()
-> Result<(), Box<dyn std::error::Error>> {
    use suertudo_identity::adapters::{refusal_of, storage_failure};
    use suertudo_identity::domain::errors::Refusal;

    // Every SQLSTATE of the schema that the catalogue names.
    let cases = [
        (
            "PT001",
            Refusal::TicketLimitReached,
            "TICKET_LIMIT_REACHED",
            409,
        ),
        ("PT002", Refusal::RaffleNotOpen, "RAFFLE_NOT_OPEN", 409),
        ("PT003", Refusal::RaffleClosed, "RAFFLE_CLOSED", 409),
        ("PT004", Refusal::NumberTaken, "NUMBER_TAKEN", 409),
        (
            "PT005",
            Refusal::TicketSizeOutOfRange,
            "TICKET_SIZE_OUT_OF_RANGE",
            422,
        ),
        ("42501", Refusal::RbacForbidden, "RBAC_FORBIDDEN", 403),
    ];
    for (sqlstate, refusal, code, status) in cases {
        assert_eq!(
            refusal_of(sqlstate),
            Some(refusal),
            "the SQLSTATE {sqlstate} is not mapped"
        );
        assert_eq!(refusal.code(), code);
        assert_eq!(refusal.status(), status);
    }

    // A SQLSTATE that the catalogue does not name is not a refusal.
    assert_eq!(refusal_of("23505"), None);

    // The SQLSTATE reaches the storage layer with its meaning.
    use suertudo_identity::ports::StorageError;
    assert!(matches!(
        storage_failure("42501", "the device can only be registered by its owner"),
        StorageError::Refused {
            refusal: Refusal::RbacForbidden,
            ..
        }
    ));
    assert!(matches!(
        storage_failure("55000", "the refresh token family is revoked"),
        StorageError::FamilyRevoked(_)
    ));
    assert!(matches!(
        storage_failure("23503", "the raffle does not exist"),
        StorageError::Missing(_)
    ));
    assert!(matches!(
        storage_failure("23505", "duplicate key value"),
        StorageError::Conflict(_)
    ));
    assert!(matches!(
        storage_failure("XX000", "something else"),
        StorageError::Backend(_)
    ));

    // And the refusal reaches the client with the code of the catalogue,
    // the status that goes with it and the message of the database in the
    // details.
    use suertudo_identity::domain::errors::IdentityError;
    use suertudo_identity::http;
    let error = IdentityError::Refused {
        refusal: Refusal::TicketLimitReached,
        message: Refusal::TicketLimitReached.message().to_string(),
        detail: "TICKET_LIMIT_REACHED: the participant reached the limit of 1 tickets".to_string(),
    };
    let answer = http::failure(&error);
    assert_eq!(answer["status_code"], 409);
    assert_eq!(answer["body"]["error"]["code"], "TICKET_LIMIT_REACHED");
    assert_eq!(
        answer["body"]["error"]["details"]["reason"],
        "TICKET_LIMIT_REACHED: the participant reached the limit of 1 tickets"
    );

    Ok(())
}

/// The device tokens die with the account.
///
/// The token of a push device is a personal datum and a credential: it
/// does not survive the deletion of the account (`FR-ID-05`).
#[tokio::test]
async fn the_anonymisation_leaves_no_device_token() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;

    let token = magic_link(&setup.deps, &unique_email()).await?;
    let session = session(&setup.deps, &token).await?;
    let access = session["access_token"].as_str().unwrap().to_string();
    let user_id = uuid::Uuid::parse_str(session["user"]["id"].as_str().unwrap())?;

    // The participant registers a device.
    let device_token = format!("device-{}", uuid::Uuid::new_v4());
    let answer = handlers::register_device(
        &setup.deps,
        request_with_token(
            json!({ "platform": "android", "token": device_token }),
            &access,
        ),
    )
    .await;
    assert_eq!(status(&answer), 201);

    let answer =
        handlers::delete_profile(&setup.deps, request_with_token(json!({}), &access)).await;
    assert_eq!(status(&answer), 202);

    // The token of the device is gone.
    let devices: i64 = sqlx::query_scalar("SELECT count(*) FROM push_devices WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&setup.owner)
        .await?;
    assert_eq!(devices, 0, "the device token survived the deletion");

    Ok(())
}

/// A participant with a session.
async fn participant(
    deps: &Arc<suertudo_identity::Deps>,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let token = magic_link(deps, &unique_email()).await?;
    let session = session(deps, &token).await?;
    let access = session["access_token"].as_str().unwrap().to_string();
    let user_id = session["user"]["id"].as_str().unwrap().to_string();
    Ok((access, user_id))
}

/// The phone that the participant has on file.
async fn with_phone(
    deps: &Arc<suertudo_identity::Deps>,
    access: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let phone = format!("+5491{:08}", uuid::Uuid::new_v4().as_u128() % 100_000_000);
    let answer = handlers::update_profile(
        deps,
        request_with_token(json!({ "phone_e164": phone }), access),
    )
    .await;
    assert_eq!(status(&answer), 200, "the phone was not saved: {answer}");
    Ok(phone)
}

/// The answer of registering a device.
async fn register_device(
    deps: &Arc<suertudo_identity::Deps>,
    access: &str,
    token: &str,
) -> serde_json::Value {
    handlers::register_device(
        deps,
        request_with_token(json!({ "platform": "android", "token": token }), access),
    )
    .await
}

/// The answer of confirming a takeover.
async fn confirm_takeover(
    deps: &Arc<suertudo_identity::Deps>,
    access: &str,
    verification_id: &str,
    code: &str,
) -> serde_json::Value {
    let mut payload = request_with_token(json!({ "code": code }), access);
    payload["path_params"] = json!({ "verification_id": verification_id });
    handlers::confirm_device_takeover(deps, payload).await
}

/// The code of a verification, out of the body of the SMS.
///
/// The code is the only run of six digits of the message: the rest of the
/// numbers of the body are the minutes of the expiry.
fn code_of(body: &str) -> Result<String, Box<dyn std::error::Error>> {
    let code = body
        .split(|c: char| !c.is_ascii_digit())
        .find(|part| part.len() == 6)
        .ok_or("the SMS does not carry a code")?;
    Ok(code.to_string())
}

/// A device that is free is registered and nothing else happens.
#[tokio::test]
async fn a_free_device_is_registered_without_a_verification()
-> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock, true).await?;
    let (access, _) = participant(&setup.deps).await?;

    let token = format!("device-{}", uuid::Uuid::new_v4());
    let answer = register_device(&setup.deps, &access, &token).await;
    assert_eq!(
        status(&answer),
        201,
        "the device was not registered: {answer}"
    );

    // No verification and no SMS: there was nothing to verify.
    assert!(
        setup.sms.sent().is_empty(),
        "a free device opened a verification"
    );
    let verifications: i64 =
        sqlx::query_scalar("SELECT count(*) FROM device_verifications WHERE device_token = $1")
            .bind(&token)
            .fetch_one(&setup.owner)
            .await?;
    assert_eq!(verifications, 0, "a free device opened a verification");

    Ok(())
}

/// The token of another participant opens a verification.
#[tokio::test]
async fn a_repeated_token_opens_a_verification() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock, true).await?;
    let (owner_access, owner_id) = participant(&setup.deps).await?;
    let _ = with_phone(&setup.deps, &owner_access).await?;
    let (claimant_access, _) = participant(&setup.deps).await?;

    let token = format!("device-{}", uuid::Uuid::new_v4());
    let answer = register_device(&setup.deps, &owner_access, &token).await;
    assert_eq!(status(&answer), 201);

    let answer = register_device(&setup.deps, &claimant_access, &token).await;
    assert_eq!(status(&answer), 202, "the takeover did not open: {answer}");
    assert_eq!(data(&answer)["status"], "pending");
    assert!(
        !data(&answer)["verification_id"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "the answer has no verification"
    );
    assert!(
        data(&answer)["expires_at"].as_i64().unwrap_or_default() > NOW,
        "the answer has no expiry"
    );

    // And the token did not move: it is still the one of its owner.
    let owner: uuid::Uuid = sqlx::query_scalar("SELECT user_id FROM push_devices WHERE token = $1")
        .bind(&token)
        .fetch_one(&setup.owner)
        .await?;
    assert_eq!(
        owner.to_string(),
        owner_id,
        "the token moved before the verification"
    );

    Ok(())
}

/// The SMS goes to the owner of the token, and not to the claimant.
///
/// This is the security property of the whole flow: if the message went to
/// the claimant, whoever stole the string of the token would receive their
/// own code and the transfer would go through.
#[tokio::test]
async fn the_sms_goes_to_the_owner_of_the_token_and_not_to_the_claimant()
-> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock, true).await?;
    let (owner_access, _) = participant(&setup.deps).await?;
    let owner_phone = with_phone(&setup.deps, &owner_access).await?;

    let (claimant_access, _) = participant(&setup.deps).await?;
    let claimant_phone = with_phone(&setup.deps, &claimant_access).await?;

    let token = format!("device-{}", uuid::Uuid::new_v4());
    let answer = register_device(&setup.deps, &owner_access, &token).await;
    assert_eq!(status(&answer), 201);

    let answer = register_device(&setup.deps, &claimant_access, &token).await;
    assert_eq!(status(&answer), 202);

    let recipient = setup.sms.last_recipient().ok_or("no SMS was sent")?;
    assert_eq!(
        recipient, owner_phone,
        "the SMS did not go to the owner of the token"
    );
    assert_ne!(recipient, claimant_phone, "the SMS went to the claimant");

    Ok(())
}

/// The right code moves the token to the claimant.
#[tokio::test]
async fn a_correct_code_transfers_the_device() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock, true).await?;
    let (owner_access, _) = participant(&setup.deps).await?;
    let _ = with_phone(&setup.deps, &owner_access).await?;
    let (claimant_access, claimant_id) = participant(&setup.deps).await?;

    let token = format!("device-{}", uuid::Uuid::new_v4());
    let answer = register_device(&setup.deps, &owner_access, &token).await;
    assert_eq!(status(&answer), 201);

    let answer = register_device(&setup.deps, &claimant_access, &token).await;
    assert_eq!(status(&answer), 202);
    let verification_id = data(&answer)["verification_id"]
        .as_str()
        .unwrap()
        .to_string();

    let code = code_of(&setup.sms.last_body().ok_or("no SMS was sent")?)?;
    let answer = confirm_takeover(&setup.deps, &claimant_access, &verification_id, &code).await;
    assert_eq!(status(&answer), 200, "the code was not accepted: {answer}");
    assert_eq!(data(&answer)["verified"], true);

    // The token moved to the claimant.
    let owner: uuid::Uuid = sqlx::query_scalar("SELECT user_id FROM push_devices WHERE token = $1")
        .bind(&token)
        .fetch_one(&setup.owner)
        .await?;
    assert_eq!(owner.to_string(), claimant_id, "the token did not move");

    Ok(())
}

/// A wrong code is refused with its own code, and the attempt is counted.
#[tokio::test]
async fn a_wrong_code_is_refused_with_its_own_code() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock, true).await?;
    let (owner_access, owner_id) = participant(&setup.deps).await?;
    let _ = with_phone(&setup.deps, &owner_access).await?;
    let (claimant_access, _) = participant(&setup.deps).await?;

    let token = format!("device-{}", uuid::Uuid::new_v4());
    let answer = register_device(&setup.deps, &owner_access, &token).await;
    assert_eq!(status(&answer), 201);

    let answer = register_device(&setup.deps, &claimant_access, &token).await;
    assert_eq!(status(&answer), 202);
    let verification_id = data(&answer)["verification_id"]
        .as_str()
        .unwrap()
        .to_string();

    // The code is wrong: the participant is doing the right thing, and the
    // answer says so.
    let answer = confirm_takeover(&setup.deps, &claimant_access, &verification_id, "000000").await;
    assert_eq!(
        status(&answer),
        400,
        "the wrong code was accepted: {answer}"
    );
    assert_eq!(error_code(&answer), "DEVICE_CODE_INVALID");

    // The attempt is counted: the refusal is a result, not an error of the
    // transaction.
    let attempts: i32 =
        sqlx::query_scalar("SELECT attempts FROM device_verifications WHERE id = $1")
            .bind(uuid::Uuid::parse_str(&verification_id)?)
            .fetch_one(&setup.owner)
            .await?;
    assert_eq!(attempts, 1, "the attempt was not counted");

    // And the token did not move.
    let owner: uuid::Uuid = sqlx::query_scalar("SELECT user_id FROM push_devices WHERE token = $1")
        .bind(&token)
        .fetch_one(&setup.owner)
        .await?;
    assert_eq!(
        owner.to_string(),
        owner_id,
        "the token moved with a wrong code"
    );

    // Five attempts: the sixth one does not reach the database, not even
    // with the right code.
    for _ in 0..4 {
        let answer =
            confirm_takeover(&setup.deps, &claimant_access, &verification_id, "000000").await;
        assert_eq!(status(&answer), 400);
    }
    let attempts: i32 =
        sqlx::query_scalar("SELECT attempts FROM device_verifications WHERE id = $1")
            .bind(uuid::Uuid::parse_str(&verification_id)?)
            .fetch_one(&setup.owner)
            .await?;
    assert_eq!(attempts, 5, "the attempts were not counted");

    let right = code_of(&setup.sms.last_body().ok_or("no SMS was sent")?)?;
    let answer = confirm_takeover(&setup.deps, &claimant_access, &verification_id, &right).await;
    assert_eq!(
        status(&answer),
        400,
        "the sixth attempt was accepted: {answer}"
    );

    Ok(())
}

/// A verification that expired is refused with its own code.
#[tokio::test]
async fn an_expired_verification_is_refused_with_its_own_code()
-> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock, true).await?;
    let (owner_access, _) = participant(&setup.deps).await?;
    let _ = with_phone(&setup.deps, &owner_access).await?;
    let (claimant_access, _) = participant(&setup.deps).await?;

    let token = format!("device-{}", uuid::Uuid::new_v4());
    let answer = register_device(&setup.deps, &owner_access, &token).await;
    assert_eq!(status(&answer), 201);

    let answer = register_device(&setup.deps, &claimant_access, &token).await;
    assert_eq!(status(&answer), 202);
    let verification_id = data(&answer)["verification_id"]
        .as_str()
        .unwrap()
        .to_string();
    let code = code_of(&setup.sms.last_body().ok_or("no SMS was sent")?)?;

    // The verification ages: the clock of the test is not the clock of the
    // database, so the row is the one that moves.
    sqlx::query(
        "UPDATE device_verifications SET expires_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(uuid::Uuid::parse_str(&verification_id)?)
    .execute(&setup.owner)
    .await?;

    let answer = confirm_takeover(&setup.deps, &claimant_access, &verification_id, &code).await;
    assert_eq!(
        status(&answer),
        409,
        "the expired code was accepted: {answer}"
    );
    assert_eq!(error_code(&answer), "DEVICE_VERIFICATION_EXPIRED");

    Ok(())
}

/// A takeover whose owner has no phone cannot be verified.
#[tokio::test]
async fn a_device_without_a_phone_cannot_be_verified() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock, true).await?;
    let (owner_access, owner_id) = participant(&setup.deps).await?;
    // The owner has no phone on file.
    let (claimant_access, _) = participant(&setup.deps).await?;

    let token = format!("device-{}", uuid::Uuid::new_v4());
    let answer = register_device(&setup.deps, &owner_access, &token).await;
    assert_eq!(status(&answer), 201);

    let answer = register_device(&setup.deps, &claimant_access, &token).await;
    assert_eq!(
        status(&answer),
        409,
        "the takeover was not refused: {answer}"
    );
    assert_eq!(error_code(&answer), "DEVICE_OWNER_UNVERIFIABLE");
    assert!(
        setup.sms.sent().is_empty(),
        "the SMS went to a participant without a phone"
    );

    // And the token did not move.
    let owner: uuid::Uuid = sqlx::query_scalar("SELECT user_id FROM push_devices WHERE token = $1")
        .bind(&token)
        .fetch_one(&setup.owner)
        .await?;
    assert_eq!(
        owner.to_string(),
        owner_id,
        "the token moved without a verification"
    );

    Ok(())
}

/// The logout hands the devices back.
#[tokio::test]
async fn the_logout_releases_the_devices() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock, true).await?;

    let token = magic_link(&setup.deps, &unique_email()).await?;
    let session = session(&setup.deps, &token).await?;
    let access = session["access_token"].as_str().unwrap().to_string();
    let refresh = session["refresh_token"].as_str().unwrap().to_string();
    let user_id = session["user"]["id"].as_str().unwrap().to_string();

    let device_token = format!("device-{}", uuid::Uuid::new_v4());
    let answer = register_device(&setup.deps, &access, &device_token).await;
    assert_eq!(status(&answer), 201);

    let devices: i64 = sqlx::query_scalar("SELECT count(*) FROM push_devices WHERE user_id = $1")
        .bind(uuid::Uuid::parse_str(&user_id)?)
        .fetch_one(&setup.owner)
        .await?;
    assert_eq!(devices, 1, "the device was not registered");

    // The logout of the session that owns the device.
    let answer = handlers::logout(&setup.deps, request(json!({ "refresh_token": refresh }))).await;
    assert_eq!(status(&answer), 200);

    // The device is gone: a device belongs to a session, not to a person.
    let devices: i64 = sqlx::query_scalar("SELECT count(*) FROM push_devices WHERE user_id = $1")
        .bind(uuid::Uuid::parse_str(&user_id)?)
        .fetch_one(&setup.owner)
        .await?;
    assert_eq!(devices, 0, "the device survived the logout");

    Ok(())
}

/// Revoking a token twice reports the second revoke as lost.
///
/// This is the mechanism that closes the window between reading a token and
/// revoking it. Two refreshes with the same token can be in flight at the
/// same time; both read the token as live, and only one may mint a
/// successor. The loser sees `false` and takes the family down.
///
/// The window itself cannot be reproduced deterministically through the
/// public interface, because it needs the two requests to interleave
/// between the read and the revoke. What is pinned here is the contract the
/// service relies on: the second revoke does not silently succeed.
#[tokio::test]
async fn revoking_a_token_twice_reports_the_second_revoke_as_lost()
-> Result<(), Box<dyn std::error::Error>> {
    use suertudo_identity::adapters::Sha256Digest;
    use suertudo_identity::ports::Digest;

    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let deps = setup.deps.clone();

    let email = unique_email();
    let token = magic_link(&deps, &email).await?;
    let session = session(&deps, &token).await?;
    let refresh = session["refresh_token"].as_str().unwrap().to_string();

    let tokens = deps.service.tokens.clone();
    let stored = tokens
        .find_by_hash(&Sha256Digest.sha256_hex(&refresh))
        .await?
        .ok_or("the refresh token is not in the store")?;
    let user_id = stored.user_id.clone().ok_or("the token has no user")?;

    assert!(
        tokens.revoke_token(&stored.id, &user_id).await?,
        "the first revoke is the one that revokes"
    );
    assert!(
        !tokens.revoke_token(&stored.id, &user_id).await?,
        "the second revoke must report that somebody else won the race"
    );

    Ok(())
}

/// The death of a family does not depend on the order of the writes.
///
/// The race of `FR-ID-04` is the one in which the reuse is detected before
/// the winner of the rotation mints its successor. The successor must not
/// survive: the family is dead, and a dead family refuses the tokens that
/// arrive after it.
///
/// The state the loser of the race finds is reproduced out of band: the
/// token is revoked (the winner of the rotation did it) and the reuse is
/// detected, and only then the winner tries to mint its successor.
#[tokio::test]
async fn the_family_dies_when_the_reuse_is_detected_before_the_successor()
-> Result<(), Box<dyn std::error::Error>> {
    use suertudo_identity::adapters::Sha256Digest;
    use suertudo_identity::ports::{Digest, StorageError};

    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let pool = &setup.owner;
    let deps = setup.deps.clone();

    let token = magic_link(&deps, &unique_email()).await?;
    let session = session(&deps, &token).await?;
    let refresh = session["refresh_token"].as_str().unwrap().to_string();

    let tokens = deps.service.tokens.clone();
    let stored = tokens
        .find_by_hash(&Sha256Digest.sha256_hex(&refresh))
        .await?
        .ok_or("the refresh token is not in the store")?;
    let user_id = stored.user_id.clone().ok_or("the token has no user")?;
    let family_id = stored.family_id.clone().ok_or("the token has no family")?;

    // The winner of the rotation revokes the token and is about to mint its
    // successor.
    assert!(
        tokens.revoke_token(&stored.id, &user_id).await?,
        "the first revoke is the one that revokes"
    );

    // The reuse is detected: the family has to die with it.
    let answer =
        handlers::refresh_session(&deps, request(json!({ "refresh_token": refresh }))).await;
    assert_eq!(status(&answer), 401);
    assert_eq!(error_code(&answer), "AUTH_REQUIRED");

    // The death is in the database, not only in the token that presented it.
    let revoked: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT revoked_at FROM token_families WHERE id = $1")
            .bind(uuid::Uuid::parse_str(&family_id)?)
            .fetch_one(pool)
            .await?;
    assert!(
        revoked.is_some(),
        "the family is alive: a successor could still be minted"
    );

    // The winner of the rotation cannot mint its successor: the death of the
    // family is durable.
    let successor = format!("successor-of-the-winner-{}", uuid::Uuid::new_v4());
    let minted = tokens
        .insert_refresh(
            &user_id,
            &family_id,
            &Sha256Digest.sha256_hex(&successor),
            NOW + 2_592_000,
        )
        .await;
    assert!(
        matches!(minted, Err(StorageError::FamilyRevoked(_))),
        "the successor entered a dead family: {minted:?}"
    );

    // And the successor is not usable.
    let answer =
        handlers::refresh_session(&deps, request(json!({ "refresh_token": successor }))).await;
    assert_eq!(status(&answer), 401);

    Ok(())
}

/// A participant with a ticket can delete its account.
///
/// `FR-ID-05`: the tickets survive with a null owner and the prizes keep
/// theirs. The write goes through the function of the schema: an
/// anonymised ticket stops being visible to the participant that
/// anonymised it, and a plain `UPDATE` is refused by Postgres.
#[tokio::test]
async fn the_account_deletion_anonymises_a_participant_with_tickets()
-> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let pool = &setup.owner;
    let deps = setup.deps.clone();

    let token = magic_link(&deps, &unique_email()).await?;
    let session = session(&deps, &token).await?;
    let access = session["access_token"].as_str().unwrap().to_string();
    let user_id = uuid::Uuid::parse_str(session["user"]["id"].as_str().unwrap())?;

    // The participant reserves a number: it has participations.
    let ticket_id = ticket_of(pool, user_id).await?;

    // The deletion runs with the role of the application: the role does not
    // own the schema, so the policies of the schema apply to it.
    let answer = handlers::delete_profile(&deps, request_with_token(json!({}), &access)).await;
    assert_eq!(status(&answer), 202, "the deletion failed: {answer}");

    // The ticket survives, without an owner.
    assert_eq!(
        ticket_owner(pool, ticket_id).await?,
        None,
        "the ticket still has an owner"
    );

    // The profile is gone and the session died with it.
    let answer = handlers::get_profile(&deps, request_with_token(json!({}), &access)).await;
    assert_eq!(status(&answer), 404);

    Ok(())
}

/// The profile keeps the notification preferences it is given.
///
/// The three channels of `docs/api-contract.md` §3.6 live in a column of
/// the schema, and the answer of the endpoint has to agree with what is
/// stored.
#[tokio::test]
async fn the_profile_stores_the_notification_preferences() -> Result<(), Box<dyn std::error::Error>>
{
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let pool = &setup.owner;
    let deps = setup.deps.clone();

    let token = magic_link(&deps, &unique_email()).await?;
    let session = session(&deps, &token).await?;
    let access = session["access_token"].as_str().unwrap().to_string();
    let user_id = uuid::Uuid::parse_str(session["user"]["id"].as_str().unwrap())?;

    let answer = handlers::update_profile(
        &deps,
        request_with_token(
            json!({
                "notification_preferences": { "push": false, "email": false, "marketing": true },
            }),
            &access,
        ),
    )
    .await;
    assert_eq!(status(&answer), 200, "the patch failed: {answer}");

    // The answer agrees with the patch.
    let preferences = &data(&answer)["notification_preferences"];
    assert_eq!(preferences["push"], false);
    assert_eq!(preferences["email"], false);
    assert_eq!(preferences["marketing"], true);

    // The column of the schema holds the preferences.
    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT notification_preferences FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(pool)
            .await?;
    assert_eq!(stored["push"], false, "the column holds {stored}");
    assert_eq!(stored["email"], false, "the column holds {stored}");
    assert_eq!(stored["marketing"], true, "the column holds {stored}");

    // The marketing choice is also in the column the rest of the platform
    // reads.
    let marketing: bool = sqlx::query_scalar("SELECT marketing_opt_in FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    assert!(marketing, "the marketing choice was not stored");

    // And a later read of the profile agrees.
    let answer = handlers::get_profile(&deps, request_with_token(json!({}), &access)).await;
    assert_eq!(status(&answer), 200);
    let preferences = &data(&answer)["notification_preferences"];
    assert_eq!(preferences["push"], false);
    assert_eq!(preferences["email"], false);
    assert_eq!(preferences["marketing"], true);

    Ok(())
}

/// Two requests for a magic link at the same time answer the same.
///
/// The endpoint answers `202` whether the address exists or not: the caller
/// cannot tell a race from a first request (`docs/api-contract.md` §3.1).
#[tokio::test]
async fn a_concurrent_magic_link_request_answers_202() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;

    // The two requests reach the storage of the link together: the store
    // waits for the second one.
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let tokens = Arc::new(BarrierTokens::new(&setup.application, barrier));
    let deps = deps_with_tokens(&setup.application, clock, true, tokens);

    let email = unique_email();
    let (first, second) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            handlers::request_magic_link(&deps, request(json!({ "email": email }))),
            handlers::request_magic_link(&deps, request(json!({ "email": email }))),
        )
    })
    .await
    .map_err(|_| "the two requests did not reach the storage of the link together")?;

    assert_eq!(status(&first), 202, "the first request failed: {first}");
    assert_eq!(status(&second), 202, "the second request failed: {second}");
    assert_eq!(data(&first)["sent"], true);
    assert_eq!(data(&second)["sent"], true);

    Ok(())
}

/// The second use of a link is refused, even under concurrency.
///
/// `FR-ID-01`: the link works once. The loser of the race answers
/// `400 AUTH_LINK_CONSUMED`, and the race does not leave an account
/// behind that nobody can use.
#[tokio::test]
async fn the_second_use_of_a_link_answers_auth_link_consumed_under_concurrency()
-> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let pool = &setup.owner;
    let deps = setup.deps.clone();

    let email = unique_email();
    let token = magic_link(&deps, &email).await?;

    // The lock holds the writes to the accounts: the two requests reach the
    // creation of the account at the same time. The reads go through.
    let lock = lock_users(pool).await?;

    let first = {
        let deps = deps.clone();
        let token = token.clone();
        tokio::spawn(async move {
            handlers::verify_magic_link(&deps, request(json!({ "token": token }))).await
        })
    };
    let second = {
        let deps = deps.clone();
        let token = token.clone();
        tokio::spawn(async move {
            handlers::verify_magic_link(&deps, request(json!({ "token": token }))).await
        })
    };

    // The race is reproduced when the two requests are at the creation of
    // the account, or when one of them already answered.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if waiting_writers(pool).await? >= 2 {
            break;
        }
        if first.is_finished() || second.is_finished() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // The lock is released: the requests go on.
    lock.commit().await?;

    let first = first.await?;
    let second = second.await?;

    let statuses = [status(&first), status(&second)];
    assert!(
        statuses.contains(&200),
        "no request succeeded: {first} {second}"
    );
    assert!(
        statuses.contains(&400),
        "the second use was not refused: {first} {second}"
    );

    // The refusal is the documented one.
    let refused = if status(&first) == 400 {
        &first
    } else {
        &second
    };
    assert_eq!(error_code(refused), "AUTH_LINK_CONSUMED");

    // The race did not leave an account behind: the only account of the
    // address is the one that got the session.
    let accounts: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE email = $1")
        .bind(&email)
        .fetch_one(pool)
        .await?;
    assert_eq!(accounts, 1, "the race left an account behind");

    let accepted = if status(&first) == 200 {
        &first
    } else {
        &second
    };
    let account: uuid::Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_one(pool)
        .await?;
    assert_eq!(
        data(accepted)["user"]["id"],
        account.to_string(),
        "the account that exists is not the one of the session"
    );

    // The other half of the rule: a verification that does not win the link
    // does not leave an account behind.
    //
    // The link dies while the verification is in flight: the verification is
    // at the creation of the account, and the link is revoked under it.
    let other = unique_email();
    let other_token = magic_link(&deps, &other).await?;

    let lock = lock_users(pool).await?;
    let verification = {
        let deps = deps.clone();
        let token = other_token.clone();
        tokio::spawn(async move {
            handlers::verify_magic_link(&deps, request(json!({ "token": token }))).await
        })
    };

    // The verification waits at the creation of the account.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if waiting_writers(pool).await? >= 1 {
            break;
        }
        if verification.is_finished() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // The link dies: the participant asked for another one.
    sqlx::query(
        "UPDATE auth_tokens SET consumed_at = now(), revoked_at = now() \
         WHERE email = $1 AND kind = 'magic_link' AND consumed_at IS NULL",
    )
    .bind(&other)
    .execute(pool)
    .await?;

    lock.commit().await?;
    let verification = verification.await?;

    let accounts: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE email = $1")
        .bind(&other)
        .fetch_one(pool)
        .await?;
    if status(&verification) == 200 {
        assert_eq!(accounts, 1, "the session has no account");
    } else {
        assert_eq!(
            accounts, 0,
            "the verification that did not win the link left an account behind"
        );
    }

    Ok(())
}

/// The deletion of an account leaves no personal data behind.
///
/// The address and the phone are personal data (`docs/PRD.md` §11.1): they
/// leave the profile, the tokens and the audit log. The audit keeps the
/// fact of the change, never the value of the data.
#[tokio::test]
async fn the_anonymisation_leaves_no_personal_data() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(TestClock::new(NOW));
    let setup = harness(clock.clone(), true).await?;
    let pool = &setup.owner;
    let deps = setup.deps.clone();

    let email = unique_email();
    let token = magic_link(&deps, &email).await?;
    let session = session(&deps, &token).await?;
    let access = session["access_token"].as_str().unwrap().to_string();
    let user_id = session["user"]["id"].as_str().unwrap().to_string();

    // The participant gives a phone number.
    let phone = format!("+5491{:08}", uuid::Uuid::new_v4().as_u128() % 100_000_000);
    let answer = handlers::update_profile(
        &deps,
        request_with_token(json!({ "phone_e164": phone }), &access),
    )
    .await;
    assert_eq!(status(&answer), 200);

    // The participant asks for another link and never uses it: the link
    // belongs to nobody and its address is personal data too.
    let answer = handlers::request_magic_link(&deps, request(json!({ "email": email }))).await;
    assert_eq!(status(&answer), 202);

    let answer = handlers::delete_profile(&deps, request_with_token(json!({}), &access)).await;
    assert_eq!(status(&answer), 202, "the deletion failed: {answer}");

    // The address is gone from the tokens.
    let addresses: Vec<String> =
        sqlx::query_scalar("SELECT email::text FROM auth_tokens WHERE email::text = $1")
            .bind(&email)
            .fetch_all(pool)
            .await?;
    assert!(
        addresses.is_empty(),
        "the address survived in the tokens: {addresses:?}"
    );

    // The phone is gone from the profile.
    let stored_phone: Option<String> =
        sqlx::query_scalar("SELECT phone_e164 FROM users WHERE id = $1")
            .bind(uuid::Uuid::parse_str(&user_id)?)
            .fetch_one(pool)
            .await?;
    assert!(
        stored_phone.is_none(),
        "the phone survived: {stored_phone:?}"
    );

    // The audit keeps the fact of the change and not the value of the data.
    let rows = audit_rows(pool, &user_id).await?;
    assert!(!rows.is_empty(), "the change is not auditable");
    for row in &rows {
        let text = row.to_string();
        assert!(
            !text.contains(&phone),
            "the audit kept the phone number: {text}"
        );
        assert!(!text.contains(&email), "the audit kept the address: {text}");
    }

    Ok(())
}
