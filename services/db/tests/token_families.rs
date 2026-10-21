//! The death of a family of refresh tokens.
//!
//! A detected reuse revokes the whole family. The revocation has to be
//! durable: a concurrent rotation can insert its successor after the
//! revocation, and a valid token would survive.
//!
//! `token_families` is the durable death: a row per family, with
//! `revoked_at`. The successor of a rotation only enters a family that is
//! not revoked, and the trigger that watches it locks the row of the family
//! to serialise the insertion against the revocation.

mod common;

use common::{ensure_app_role, has_message, seed_user, test_pool, with_app_role};
use uuid::Uuid;

/// A hash of a token that no other test used.
///
/// The hash is unique in the database (`auth_tokens_hash_idx`), and the tests
/// do not clean up: a fixed hash would break the second run of the test.
fn unique_hash() -> Vec<u8> {
    Uuid::new_v4().as_bytes().to_vec()
}

/// A dead family refuses a successor.
///
/// The test runs with the role of the application: the owner of the tables
/// bypasses the policies, and the guarantee has to hold for the role that
/// connects in production.
#[tokio::test]
async fn a_dead_family_refuses_a_successor() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let user_id = seed_user(&pool).await?;

    // The family is created when the session starts.
    let family_id: Uuid = with_app_role(&pool, None, Some(user_id), |tx| {
        Box::pin(async move {
            let id: Uuid =
                sqlx::query_scalar("INSERT INTO token_families (user_id) VALUES ($1) RETURNING id")
                    .bind(user_id)
                    .fetch_one(&mut **tx)
                    .await?;
            Ok(id)
        })
    })
    .await?;

    // The first token of the family.
    with_app_role(&pool, None, Some(user_id), |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO auth_tokens (user_id, email, kind, token_hash, family_id, expires_at)
                VALUES ($1, 'family@example.test', 'refresh', $2, $3, now() + interval '1 day')
                "#,
            )
            .bind(user_id)
            .bind(unique_hash())
            .bind(family_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;

    // The reuse is detected: the family dies.
    with_app_role(&pool, None, Some(user_id), |tx| {
        Box::pin(async move {
            sqlx::query(
                "UPDATE token_families SET revoked_at = now(), revoke_reason = 'reuse' WHERE id = $1",
            )
            .bind(family_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;

    // The tokens of the family go with it.
    let alive: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM auth_tokens WHERE family_id = $1 AND revoked_at IS NULL",
    )
    .bind(family_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(alive, 0, "the token of the dead family was alive");

    // The successor of the rotation does not enter a dead family.
    let err = match with_app_role(&pool, None, Some(user_id), |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO auth_tokens (user_id, email, kind, token_hash, family_id, expires_at)
                VALUES ($1, 'family@example.test', 'refresh', $2, $3, now() + interval '1 day')
                "#,
            )
            .bind(user_id)
            .bind(unique_hash())
            .bind(family_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(()) => return Err("the successor of a dead family was accepted".into()),
        Err(err) => err,
    };

    assert!(
        has_message(&err, "the refresh token family is revoked"),
        "unexpected error: {err}"
    );

    // The refusal is distinguishable from the "the participant does not
    // exist": the service maps them to different errors.
    let sqlstate = err
        .as_database_error()
        .and_then(|db| db.code())
        .map(|code| code.into_owned());
    assert_eq!(
        sqlstate.as_deref(),
        Some("55000"),
        "the refusal of a dead family is not distinguishable"
    );

    Ok(())
}

/// A live family accepts the successor of a rotation.
#[tokio::test]
async fn a_live_family_accepts_the_successor() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let user_id = seed_user(&pool).await?;

    let family_id: Uuid = with_app_role(&pool, None, Some(user_id), |tx| {
        Box::pin(async move {
            let id: Uuid =
                sqlx::query_scalar("INSERT INTO token_families (user_id) VALUES ($1) RETURNING id")
                    .bind(user_id)
                    .fetch_one(&mut **tx)
                    .await?;
            Ok(id)
        })
    })
    .await?;

    // The first token of the family.
    with_app_role(&pool, None, Some(user_id), |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO auth_tokens (user_id, email, kind, token_hash, family_id, expires_at)
                VALUES ($1, 'family@example.test', 'refresh', $2, $3, now() + interval '1 day')
                "#,
            )
            .bind(user_id)
            .bind(unique_hash())
            .bind(family_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;

    // The rotation revokes the presented token before the successor.
    with_app_role(&pool, None, Some(user_id), |tx| {
        Box::pin(async move {
            sqlx::query("UPDATE auth_tokens SET revoked_at = now() WHERE family_id = $1")
                .bind(family_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    // The successor enters the family, which is alive.
    with_app_role(&pool, None, Some(user_id), |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO auth_tokens (user_id, email, kind, token_hash, family_id, expires_at)
                VALUES ($1, 'family@example.test', 'refresh', $2, $3, now() + interval '1 day')
                "#,
            )
            .bind(user_id)
            .bind(unique_hash())
            .bind(family_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;

    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM auth_tokens WHERE family_id = $1 AND revoked_at IS NULL",
    )
    .bind(family_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(live, 1, "the successor did not enter the live family");

    Ok(())
}
