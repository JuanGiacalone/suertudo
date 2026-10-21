// A failing assertion should stop the run with a readable message: the lint
// exists to keep the panics out of the production paths, not out of the
// tests.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! La toma de control de un dispositivo, verificada por SMS.
//!
//! El token de un dispositivo identifica **un teléfono**, no a una persona.
//! Repetirlo es un caso real (la misma persona, una segunda cuenta, el
//! mismo teléfono) y también es el intento de robo que el defecto `F-3`
//! describía. La diferencia entre los dos es la verificación: el SMS va al
//! teléfono del dueño **actual** del token, y la transferencia sólo ocurre
//! después de que se confirme.
//!
//! Los tests corren con el rol de la aplicación: el dueño de las tablas
//! esquiva las políticas de RLS por diseño, y una garantía de aislamiento
//! que sólo se ve con el dueño no es una garantía.

mod common;

use common::{
    ensure_app_role, seed_user, test_pool, with_app_participant, with_app_role, with_app_system,
};
use uuid::Uuid;

/// The SQLSTATE of an error, when it comes from the database.
fn sqlstate(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|db| db.code())
        .map(|code| code.into_owned())
}

/// The SHA-256 of a code, computed by the database itself.
///
/// The test does not carry a second implementation of the hash: it asks
/// pgcrypto, the same function the schema uses.
async fn hash_of(pool: &sqlx::PgPool, code: &[u8]) -> Result<Vec<u8>, sqlx::Error> {
    sqlx::query_scalar("SELECT digest($1::bytea, 'sha256')")
        .bind(code)
        .fetch_one(pool)
        .await
}

/// A token that no other test used.
fn unique_token() -> String {
    format!("tok-{}", Uuid::new_v4())
}

/// A code that no other test used.
fn unique_code() -> Vec<u8> {
    Uuid::new_v4().simple().to_string().as_bytes()[..6].to_vec()
}

/// A phone that no other test used.
fn unique_phone() -> String {
    format!("+54911{}", Uuid::new_v4().simple())
}

/// The owner of the token of a device.
async fn owner_of(pool: &sqlx::PgPool, token: &str) -> Option<Uuid> {
    sqlx::query_scalar("SELECT user_id FROM push_devices WHERE token = $1")
        .bind(token)
        .fetch_optional(pool)
        .await
        .expect("the device row is readable")
}

/// The owner of a free token registers it and no verification is opened.
///
/// The common path: the token is new, nobody has to prove anything, and the
/// participant gets its push.
#[tokio::test]
async fn a_free_token_is_registered_without_a_verification()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let user_id = seed_user(&pool).await?;
    let token = unique_token();
    let request_token = token.clone();
    let hash = hash_of(&pool, b"123456").await?;

    let verification_id: Option<Uuid> = with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT request_device_takeover($1, 'android', '1.0', $2, $3, now() + interval '10 minutes')",
            )
            .bind(&request_token)
            .bind(user_id)
            .bind(&hash)
            .fetch_one(&mut **tx)
            .await?;
            Ok(id)
        })
    })
    .await?;

    assert!(
        verification_id.is_none(),
        "a free token opened a verification: {verification_id:?}"
    );
    assert_eq!(
        owner_of(&pool, &token).await,
        Some(user_id),
        "the device was not registered for the participant"
    );

    let verifications: i64 =
        sqlx::query_scalar("SELECT count(*) FROM device_verifications WHERE device_token = $1")
            .bind(&token)
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        verifications, 0,
        "a free token left a verification in the table"
    );

    Ok(())
}

/// The token of another participant opens a verification and does not move.
///
/// The owner of the token does not change until somebody confirms: this is
/// the guarantee that stops the theft of the defect `F-3`.
#[tokio::test]
async fn a_token_of_another_participant_opens_a_verification()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let claimant_id = seed_user(&pool).await?;
    let token = unique_token();
    let owner_token = token.clone();
    let claim_token = token.clone();
    let hash = hash_of(&pool, b"654321").await?;

    // The owner registers the device.
    with_app_participant(&pool, owner_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT register_push_device($1, 'android', $2, '1.0')")
                .bind(owner_id)
                .bind(&owner_token)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    // The claimant asks for the same token.
    let verification_id: Option<Uuid> = with_app_participant(&pool, claimant_id, |tx| {
        Box::pin(async move {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT request_device_takeover($1, 'android', '1.0', $2, $3, now() + interval '10 minutes')",
            )
            .bind(&claim_token)
            .bind(claimant_id)
            .bind(&hash)
            .fetch_one(&mut **tx)
            .await?;
            Ok(id)
        })
    })
    .await?;

    let verification_id = verification_id.expect("the takeover did not open a verification");
    assert_eq!(
        owner_of(&pool, &token).await,
        Some(owner_id),
        "the token changed hands before the verification"
    );

    let (status, current_owner, claimant, stored_hash): (String, Uuid, Uuid, Vec<u8>) =
        sqlx::query_as(
            "SELECT status, current_owner_id, claimant_id, code_hash
               FROM device_verifications WHERE id = $1",
        )
        .bind(verification_id)
        .fetch_one(&pool)
        .await?;

    assert_eq!(status, "pending", "the verification is not pending");
    assert_eq!(
        current_owner, owner_id,
        "the verification does not name the owner of the token"
    );
    assert_eq!(
        claimant, claimant_id,
        "the verification is not the claimant's"
    );
    assert_eq!(
        stored_hash,
        hash_of(&pool, b"654321").await?,
        "the code was stored in clear"
    );

    Ok(())
}

/// The verification belongs to the claimant: nobody else confirms it.
///
/// Neither the owner of the token (confirming would be stealing the phone
/// of the other) nor a third participant.
#[tokio::test]
async fn the_verification_belongs_to_the_claimant_only() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let claimant_id = seed_user(&pool).await?;
    let stranger_id = seed_user(&pool).await?;
    let token = unique_token();
    let code = unique_code();
    let owner_code = code.clone();
    let stranger_code = code.clone();
    let hash = hash_of(&pool, &code).await?;

    let verification_id = open_verification(&pool, owner_id, claimant_id, &token, &hash).await?;

    // The owner of the token tries to confirm it.
    let err = match with_app_participant(&pool, owner_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT confirm_device_takeover($1, $2, $3)")
                .bind(verification_id)
                .bind(owner_id)
                .bind(&owner_code)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the owner of the token confirmed the verification".into()),
        Err(err) => err,
    };
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "unexpected error: {err}"
    );

    // A third participant tries to confirm it.
    let err = match with_app_participant(&pool, stranger_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT confirm_device_takeover($1, $2, $3)")
                .bind(verification_id)
                .bind(stranger_id)
                .bind(&stranger_code)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("a stranger confirmed the verification".into()),
        Err(err) => err,
    };
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "unexpected error: {err}"
    );

    assert_eq!(
        owner_of(&pool, &token).await,
        Some(owner_id),
        "the token changed hands without the claimant"
    );

    Ok(())
}

/// A wrong code does not transfer the token, and it counts the attempt.
///
/// The attempt counter is the reason the function returns `false` instead of
/// raising: a raised exception aborts the transaction and the counter would
/// be lost.
#[tokio::test]
async fn a_wrong_code_does_not_transfer_the_token() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let claimant_id = seed_user(&pool).await?;
    let token = unique_token();
    let code = unique_code();
    let hash = hash_of(&pool, &code).await?;

    let verification_id = open_verification(&pool, owner_id, claimant_id, &token, &hash).await?;

    let confirmed: bool = with_app_participant(&pool, claimant_id, |tx| {
        Box::pin(async move {
            let ok: bool = sqlx::query_scalar("SELECT confirm_device_takeover($1, $2, $3)")
                .bind(verification_id)
                .bind(claimant_id)
                .bind(b"000000".to_vec())
                .fetch_one(&mut **tx)
                .await?;
            Ok(ok)
        })
    })
    .await?;

    assert!(!confirmed, "a wrong code confirmed the verification");
    assert_eq!(
        owner_of(&pool, &token).await,
        Some(owner_id),
        "a wrong code transferred the token"
    );

    let attempts: i32 =
        sqlx::query_scalar("SELECT attempts FROM device_verifications WHERE id = $1")
            .bind(verification_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(attempts, 1, "the wrong attempt was not counted: {attempts}");

    Ok(())
}

/// The right code transfers the token, and only then.
#[tokio::test]
async fn a_correct_code_transfers_the_token() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let claimant_id = seed_user(&pool).await?;
    let token = unique_token();
    let code = unique_code();
    let second_code = code.clone();
    let hash = hash_of(&pool, &code).await?;

    let verification_id = open_verification(&pool, owner_id, claimant_id, &token, &hash).await?;

    let confirmed: bool = with_app_participant(&pool, claimant_id, |tx| {
        Box::pin(async move {
            let ok: bool = sqlx::query_scalar("SELECT confirm_device_takeover($1, $2, $3)")
                .bind(verification_id)
                .bind(claimant_id)
                .bind(&code)
                .fetch_one(&mut **tx)
                .await?;
            Ok(ok)
        })
    })
    .await?;

    assert!(confirmed, "the right code did not confirm the verification");
    assert_eq!(
        owner_of(&pool, &token).await,
        Some(claimant_id),
        "the token did not change hands after the verification"
    );

    let (status, confirmed_at): (String, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("SELECT status, confirmed_at FROM device_verifications WHERE id = $1")
            .bind(verification_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(status, "confirmed", "the verification is not confirmed");
    assert!(
        confirmed_at.is_some(),
        "the confirmed verification has no date of confirmation"
    );

    // A confirmed verification cannot be confirmed again.
    let again: bool = with_app_participant(&pool, claimant_id, |tx| {
        Box::pin(async move {
            let ok: bool = sqlx::query_scalar("SELECT confirm_device_takeover($1, $2, $3)")
                .bind(verification_id)
                .bind(claimant_id)
                .bind(&second_code)
                .fetch_one(&mut **tx)
                .await?;
            Ok(ok)
        })
    })
    .await?;
    assert!(
        !again,
        "a confirmed verification was confirmed a second time"
    );
    assert_eq!(
        owner_of(&pool, &token).await,
        Some(claimant_id),
        "the second confirmation moved the token"
    );

    Ok(())
}

/// An expired verification cannot be confirmed.
///
/// The code that arrives late is not a wrong code: it is a verification that
/// venció, and the client has to be able to tell the two apart.
#[tokio::test]
async fn an_expired_verification_cannot_be_confirmed() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let claimant_id = seed_user(&pool).await?;
    let token = unique_token();
    let code = unique_code();
    let hash = hash_of(&pool, &code).await?;

    let verification_id = open_verification(&pool, owner_id, claimant_id, &token, &hash).await?;

    // The verification venció.
    sqlx::query(
        "UPDATE device_verifications SET expires_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(verification_id)
    .execute(&pool)
    .await?;

    let err = match with_app_participant(&pool, claimant_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT confirm_device_takeover($1, $2, $3)")
                .bind(verification_id)
                .bind(claimant_id)
                .bind(&code)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("an expired verification was confirmed".into()),
        Err(err) => err,
    };

    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("PT006"),
        "unexpected error: {err}"
    );
    assert_eq!(
        owner_of(&pool, &token).await,
        Some(owner_id),
        "an expired verification transferred the token"
    );

    Ok(())
}

/// A second request reuses the pending verification.
///
/// The flow does not leave a pile of pending rows for the same token: the
/// second request refreshes the code and the expiry of the first.
#[tokio::test]
async fn a_second_request_reuses_the_pending_verification() -> Result<(), Box<dyn std::error::Error>>
{
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let claimant_id = seed_user(&pool).await?;
    let token = unique_token();
    let first_hash = hash_of(&pool, b"111111").await?;
    let second_hash = hash_of(&pool, b"222222").await?;
    let expected_hash = second_hash.clone();

    let first = open_verification(&pool, owner_id, claimant_id, &token, &first_hash).await?;

    // The claimant asks again, with a new code and a new expiry.
    let request_token = token.clone();
    let second: Option<Uuid> = with_app_participant(&pool, claimant_id, |tx| {
        Box::pin(async move {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT request_device_takeover($1, 'ios', '2.0', $2, $3, now() + interval '30 minutes')",
            )
            .bind(&request_token)
            .bind(claimant_id)
            .bind(&second_hash)
            .fetch_one(&mut **tx)
            .await?;
            Ok(id)
        })
    })
    .await?;

    let second = second.expect("the second request did not open a verification");
    assert_eq!(
        second, first,
        "the second request opened another verification"
    );

    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM device_verifications
          WHERE device_token = $1 AND claimant_id = $2 AND status = 'pending'",
    )
    .bind(&token)
    .bind(claimant_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(pending, 1, "the token piled up pending verifications");

    let stored_hash: Vec<u8> =
        sqlx::query_scalar("SELECT code_hash FROM device_verifications WHERE id = $1")
            .bind(first)
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        stored_hash, expected_hash,
        "the second request did not refresh the code"
    );

    // The database does not let a second pending row exist for the same
    // pair: the index is the guarantee, not only the query that reuses.
    let err = sqlx::query(
        r#"
        INSERT INTO device_verifications
            (device_token, current_owner_id, claimant_id, code_hash, expires_at)
        VALUES ($1, $2, $3, $4, now() + interval '10 minutes')
        "#,
    )
    .bind(&token)
    .bind(owner_id)
    .bind(claimant_id)
    .bind(&first_hash)
    .execute(&pool)
    .await
    .expect_err("a second pending verification for the same token was accepted");
    assert_eq!(
        err.as_database_error().and_then(|db| db.code()).as_deref(),
        Some("23505"),
        "unexpected error: {err}"
    );

    Ok(())
}

/// The logout releases the devices of the participant.
///
/// A device belongs to a session, not to a person: when the session closes,
/// the token goes back to the pool and the next account of the same phone
/// registers it without a fight.
#[tokio::test]
async fn the_logout_releases_the_devices() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let user_id = seed_user(&pool).await?;
    let other_id = seed_user(&pool).await?;
    let token = unique_token();
    let other_token = unique_token();
    let owner_token = token.clone();
    let owner_other_token = other_token.clone();

    with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT register_push_device($1, 'android', $2, '1.0')")
                .bind(user_id)
                .bind(&owner_token)
                .execute(&mut **tx)
                .await?;
            sqlx::query("SELECT register_push_device($1, 'ios', $2, '1.1')")
                .bind(user_id)
                .bind(&owner_other_token)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    let released: i32 = with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            let count: i32 = sqlx::query_scalar("SELECT release_user_devices($1)")
                .bind(user_id)
                .fetch_one(&mut **tx)
                .await?;
            Ok(count)
        })
    })
    .await?;

    assert_eq!(released, 2, "the logout did not release both devices");
    assert_eq!(
        owner_of(&pool, &token).await,
        None,
        "the device survived the logout"
    );

    // The token is free: the next account of the same phone registers it
    // without a verification.
    let next_token = token.clone();
    let hash = hash_of(&pool, b"333333").await?;
    let verification_id: Option<Uuid> = with_app_participant(&pool, other_id, |tx| {
        Box::pin(async move {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT request_device_takeover($1, 'android', '1.0', $2, $3, now() + interval '10 minutes')",
            )
            .bind(&next_token)
            .bind(other_id)
            .bind(&hash)
            .fetch_one(&mut **tx)
            .await?;
            Ok(id)
        })
    })
    .await?;

    assert!(
        verification_id.is_none(),
        "the released token opened a verification: {verification_id:?}"
    );
    assert_eq!(
        owner_of(&pool, &token).await,
        Some(other_id),
        "the released token did not go to the next account"
    );

    Ok(())
}

/// A participant sees their own verification and nobody else's.
///
/// The test runs as the role of the application: the owner of the tables
/// bypasses the policies, and the isolation would not be proven.
#[tokio::test]
async fn a_participant_cannot_see_the_verification_of_another()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let claimant_id = seed_user(&pool).await?;
    let stranger_id = seed_user(&pool).await?;
    let token = unique_token();
    let hash = hash_of(&pool, b"444444").await?;

    let verification_id = open_verification(&pool, owner_id, claimant_id, &token, &hash).await?;

    // The claimant sees its own verification.
    let visible: Vec<Uuid> = with_app_participant(&pool, claimant_id, |tx| {
        Box::pin(async move {
            let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM device_verifications")
                .fetch_all(&mut **tx)
                .await?;
            Ok(ids)
        })
    })
    .await?;
    assert!(
        visible.contains(&verification_id),
        "the claimant does not see its own verification"
    );

    // A third participant does not.
    let visible: Vec<Uuid> = with_app_participant(&pool, stranger_id, |tx| {
        Box::pin(async move {
            let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM device_verifications")
                .fetch_all(&mut **tx)
                .await?;
            Ok(ids)
        })
    })
    .await?;
    assert!(
        !visible.contains(&verification_id),
        "a participant read the verification of another participant"
    );

    // The platform sees all of them.
    let visible: Vec<Uuid> = with_app_system(&pool, |tx| {
        Box::pin(async move {
            let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM device_verifications")
                .fetch_all(&mut **tx)
                .await?;
            Ok(ids)
        })
    })
    .await?;
    assert!(
        visible.contains(&verification_id),
        "the platform does not see the verification"
    );

    Ok(())
}

/// A verification that went stale cannot take a token that changed hands.
///
/// The owner released the device at the logout, the token was free, and
/// another account took it. The verification of the first claimant is older
/// than that: confirming it would take the device away from the account that
/// holds it today.
#[tokio::test]
async fn a_stale_verification_cannot_take_a_token_that_changed_hands()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let claimant_id = seed_user(&pool).await?;
    let new_owner_id = seed_user(&pool).await?;
    let token = unique_token();
    let code = unique_code();
    let hash = hash_of(&pool, &code).await?;

    let verification_id = open_verification(&pool, owner_id, claimant_id, &token, &hash).await?;

    // The owner logs out and the token goes back to the pool.
    with_app_participant(&pool, owner_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT release_user_devices($1)")
                .bind(owner_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    // Another account of the same phone takes the token.
    let new_token = token.clone();
    let new_hash = hash_of(&pool, b"666666").await?;
    with_app_participant(&pool, new_owner_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                "SELECT request_device_takeover($1, 'android', '1.0', $2, $3, now() + interval '10 minutes')",
            )
            .bind(&new_token)
            .bind(new_owner_id)
            .bind(&new_hash)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;

    // The claimant confirms the verification that is now stale.
    let confirmed: bool = with_app_participant(&pool, claimant_id, |tx| {
        Box::pin(async move {
            let ok: bool = sqlx::query_scalar("SELECT confirm_device_takeover($1, $2, $3)")
                .bind(verification_id)
                .bind(claimant_id)
                .bind(&code)
                .fetch_one(&mut **tx)
                .await?;
            Ok(ok)
        })
    })
    .await?;

    assert!(
        !confirmed,
        "a stale verification confirmed itself and took the device"
    );
    assert_eq!(
        owner_of(&pool, &token).await,
        Some(new_owner_id),
        "a stale verification took the device of the account that holds it"
    );

    Ok(())
}

/// A refused verification cannot be confirmed.
///
/// The owner of the device can refuse the takeover by not answering. The
/// verification that leaves the flow is `refused`, and a refused
/// verification is not a pending one: confirming it does nothing.
#[tokio::test]
async fn a_refused_verification_cannot_be_confirmed() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let claimant_id = seed_user(&pool).await?;
    let token = unique_token();
    let code = unique_code();
    let hash = hash_of(&pool, &code).await?;

    let verification_id = open_verification(&pool, owner_id, claimant_id, &token, &hash).await?;

    // The owner refused: the verification leaves the flow pending.
    sqlx::query("UPDATE device_verifications SET status = 'refused' WHERE id = $1")
        .bind(verification_id)
        .execute(&pool)
        .await?;

    let confirmed: bool = with_app_participant(&pool, claimant_id, |tx| {
        Box::pin(async move {
            let ok: bool = sqlx::query_scalar("SELECT confirm_device_takeover($1, $2, $3)")
                .bind(verification_id)
                .bind(claimant_id)
                .bind(&code)
                .fetch_one(&mut **tx)
                .await?;
            Ok(ok)
        })
    })
    .await?;

    assert!(
        !confirmed,
        "a refused verification confirmed itself and took the device"
    );
    assert_eq!(
        owner_of(&pool, &token).await,
        Some(owner_id),
        "a refused verification took the device"
    );

    Ok(())
}

/// The phone of the current owner is only for the claimant.
///
/// The SMS goes to the phone of the participant that owns the token **today**.
/// That phone cannot be read otherwise: `users.phone_e164` is behind the
/// policies of `users`, and a participant only sees its own row. The function
/// is the path, and it opens only for the claimant of the verification.
#[tokio::test]
async fn the_phone_of_the_current_owner_is_only_for_the_claimant()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let claimant_id = seed_user(&pool).await?;
    let stranger_id = seed_user(&pool).await?;
    let token = unique_token();
    let phone = unique_phone();
    let code = unique_code();
    let hash = hash_of(&pool, &code).await?;

    // The owner of the token has a phone on file.
    sqlx::query("UPDATE users SET phone_e164 = $1 WHERE id = $2")
        .bind(&phone)
        .bind(owner_id)
        .execute(&pool)
        .await?;

    let verification_id = open_verification(&pool, owner_id, claimant_id, &token, &hash).await?;

    // The claimant gets the phone of the current owner.
    let got: Option<String> = with_app_participant(&pool, claimant_id, |tx| {
        Box::pin(async move {
            let phone: Option<String> = sqlx::query_scalar("SELECT device_takeover_phone($1, $2)")
                .bind(verification_id)
                .bind(claimant_id)
                .fetch_one(&mut **tx)
                .await?;
            Ok(phone)
        })
    })
    .await?;
    assert_eq!(
        got.as_deref(),
        Some(phone.as_str()),
        "the claimant did not get the phone of the current owner"
    );

    // The owner of the token does not get it: it is the claimant's request.
    assert_eq!(
        phone_refusal(&pool, owner_id, verification_id, owner_id)
            .await?
            .as_deref(),
        Some("42501"),
        "the owner of the token got the phone"
    );

    // A third participant does not get it.
    assert_eq!(
        phone_refusal(&pool, stranger_id, verification_id, stranger_id)
            .await?
            .as_deref(),
        Some("42501"),
        "a stranger got the phone of the current owner"
    );

    // The claimant does not get it in the name of another participant: the
    // request is for the participant that asks.
    let err = match with_app_participant(&pool, claimant_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT device_takeover_phone($1, $2)")
                .bind(verification_id)
                .bind(stranger_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the claimant asked the phone in the name of another".into()),
        Err(err) => err,
    };
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "unexpected error: {err}"
    );

    // A caller with no context does not get it.
    let err = match with_app_role(&pool, None, None, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT device_takeover_phone($1, $2)")
                .bind(verification_id)
                .bind(claimant_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("a caller with no context got the phone".into()),
        Err(err) => err,
    };
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "unexpected error: {err}"
    );

    // The platform gets it: it is the one that sends the SMS.
    let got: Option<String> = with_app_system(&pool, |tx| {
        Box::pin(async move {
            let phone: Option<String> = sqlx::query_scalar("SELECT device_takeover_phone($1, $2)")
                .bind(verification_id)
                .bind(claimant_id)
                .fetch_one(&mut **tx)
                .await?;
            Ok(phone)
        })
    })
    .await?;
    assert_eq!(
        got.as_deref(),
        Some(phone.as_str()),
        "the platform did not get the phone of the current owner"
    );

    // A current owner with no phone on file: there is nobody to reach over
    // SMS, and the function says so instead of inventing a phone.
    let silent_owner_id = seed_user(&pool).await?;
    let silent_claimant_id = seed_user(&pool).await?;
    let silent_token = unique_token();
    let silent_hash = hash_of(&pool, b"777777").await?;
    let silent_verification_id = open_verification(
        &pool,
        silent_owner_id,
        silent_claimant_id,
        &silent_token,
        &silent_hash,
    )
    .await?;

    let got: Option<String> = with_app_participant(&pool, silent_claimant_id, |tx| {
        Box::pin(async move {
            let phone: Option<String> = sqlx::query_scalar("SELECT device_takeover_phone($1, $2)")
                .bind(silent_verification_id)
                .bind(silent_claimant_id)
                .fetch_one(&mut **tx)
                .await?;
            Ok(phone)
        })
    })
    .await?;
    assert!(
        got.is_none(),
        "a participant without a phone on file got one: {got:?}"
    );

    Ok(())
}

/// The functions bind the caller: a stranger cannot open a takeover.
///
/// Without the binding, any participant could open a verification on a
/// device of another, and the SMS would go to a phone that is not theirs.
#[tokio::test]
async fn a_caller_that_is_not_the_participant_is_refused() -> Result<(), Box<dyn std::error::Error>>
{
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let claimant_id = seed_user(&pool).await?;
    let stranger_id = seed_user(&pool).await?;
    let token = unique_token();
    let hash = hash_of(&pool, b"555555").await?;

    let verification_id = open_verification(&pool, owner_id, claimant_id, &token, &hash).await?;

    // The stranger asks for a takeover in the name of the claimant.
    let err = match with_app_participant(&pool, stranger_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                "SELECT request_device_takeover($1, 'android', '1.0', $2, $3, now() + interval '10 minutes')",
            )
            .bind(&token)
            .bind(claimant_id)
            .bind(&hash)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("a stranger opened a takeover in the name of another".into()),
        Err(err) => err,
    };
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "unexpected error: {err}"
    );

    // The stranger confirms a verification that is not theirs.
    let err = match with_app_participant(&pool, stranger_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT confirm_device_takeover($1, $2, $3)")
                .bind(verification_id)
                .bind(claimant_id)
                .bind(b"555555".to_vec())
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("a stranger confirmed a takeover of another".into()),
        Err(err) => err,
    };
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "unexpected error: {err}"
    );

    Ok(())
}

/// The devices of another participant cannot be released.
#[tokio::test]
async fn a_stranger_cannot_release_the_devices_of_another() -> Result<(), Box<dyn std::error::Error>>
{
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let stranger_id = seed_user(&pool).await?;
    let token = unique_token();
    let owner_token = token.clone();

    with_app_participant(&pool, owner_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT register_push_device($1, 'android', $2, '1.0')")
                .bind(owner_id)
                .bind(&owner_token)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    let err = match with_app_participant(&pool, stranger_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT release_user_devices($1)")
                .bind(owner_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("a stranger released the devices of another".into()),
        Err(err) => err,
    };
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "unexpected error: {err}"
    );
    assert_eq!(
        owner_of(&pool, &token).await,
        Some(owner_id),
        "the devices of another participant were released"
    );

    Ok(())
}

/// The SQLSTATE of the refusal of `device_takeover_phone`, for a caller with
/// the context of a participant.
///
/// The caller asks for the phone of the verification, in the name of
/// `asked_for_id`.
async fn phone_refusal(
    pool: &sqlx::PgPool,
    caller_id: Uuid,
    verification_id: Uuid,
    asked_for_id: Uuid,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let err = match with_app_participant(pool, caller_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT device_takeover_phone($1, $2)")
                .bind(verification_id)
                .bind(asked_for_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the phone of the device was handed to a stranger".into()),
        Err(err) => err,
    };

    Ok(sqlstate(&err))
}

/// Opens a verification: the owner registers the token and the claimant
/// asks for it.
///
/// Returns the id of the verification that was opened.
async fn open_verification(
    pool: &sqlx::PgPool,
    owner_id: Uuid,
    claimant_id: Uuid,
    token: &str,
    hash: &[u8],
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let owner_token = token.to_string();
    with_app_participant(pool, owner_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT register_push_device($1, 'android', $2, '1.0')")
                .bind(owner_id)
                .bind(&owner_token)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    let claim_token = token.to_string();
    let hash = hash.to_vec();
    let id: Option<Uuid> = with_app_participant(pool, claimant_id, |tx| {
        Box::pin(async move {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT request_device_takeover($1, 'android', '1.0', $2, $3, now() + interval '10 minutes')",
            )
            .bind(&claim_token)
            .bind(claimant_id)
            .bind(&hash)
            .fetch_one(&mut **tx)
            .await?;
            Ok(id)
        })
    })
    .await?;

    Ok(id.expect("the takeover did not open a verification"))
}
