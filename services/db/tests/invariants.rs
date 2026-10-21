//! The invariants of the product, proven against a real database.
//!
//! Every test here observes the database rejecting a bad write. None of them
//! checks a check in Rust: the guarantee has to survive a concurrent request,
//! a retry and a bug in the handler, and only a constraint does that.

mod common;

use common::{
    has_message, is_limit_reached, is_unique_violation, seed_business, seed_raffle, seed_ticket,
    seed_user, test_pool, unique_code, with_participant,
};
use suertudo_db::repositories::reserve_ticket;
use uuid::Uuid;

/// `UNIQUE (raffle_id, number)`: a number is reserved once per raffle.
#[tokio::test]
async fn a_number_can_only_be_reserved_once_per_raffle() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;
    let first_user = seed_user(&pool).await?;
    let second_user = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;

    let first = seed_ticket(&pool, raffle_id, business_id, first_user).await?;
    sqlx::query("INSERT INTO ticket_numbers (ticket_id, raffle_id, number) VALUES ($1, $2, 7)")
        .bind(first)
        .bind(raffle_id)
        .execute(&pool)
        .await?;

    let second = seed_ticket(&pool, raffle_id, business_id, second_user).await?;
    let result =
        sqlx::query("INSERT INTO ticket_numbers (ticket_id, raffle_id, number) VALUES ($1, $2, 7)")
            .bind(second)
            .bind(raffle_id)
            .execute(&pool)
            .await;

    let err = match result {
        Ok(_) => return Err("the second reservation of the number 7 was accepted".into()),
        Err(err) => err,
    };
    assert!(
        is_unique_violation(&err, "ticket_numbers_raffle_id_number_key"),
        "unexpected error: {err}"
    );

    // The rejected write left nothing behind.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM ticket_numbers WHERE raffle_id = $1")
        .bind(raffle_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1, "the rejected reservation left a number behind");

    Ok(())
}

/// `UNIQUE (raffle_id)` on `draws`: a raffle is drawn exactly once.
#[tokio::test]
async fn a_raffle_is_drawn_only_once() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "closed").await?;

    let insert_draw = "INSERT INTO draws (raffle_id, business_id, seed_commitment, seed_ciphertext,
                               derivation_spec, pool_size)
        VALUES ($1, $2, 'commitment', $3, '{\"hash\":\"hmac-sha256\"}'::jsonb, 1000)";

    sqlx::query(insert_draw)
        .bind(raffle_id)
        .bind(business_id)
        .bind(vec![0u8; 32])
        .execute(&pool)
        .await?;

    let result = sqlx::query(insert_draw)
        .bind(raffle_id)
        .bind(business_id)
        .bind(vec![0u8; 32])
        .execute(&pool)
        .await;
    let err = match result {
        Ok(_) => return Err("the second draw of the raffle was accepted".into()),
        Err(err) => err,
    };
    assert!(
        is_unique_violation(&err, "draws_raffle_id_key"),
        "unexpected error: {err}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM draws WHERE raffle_id = $1")
        .bind(raffle_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1, "the rejected draw left a second row behind");

    Ok(())
}

/// The partial unique index on `tickets (raffle_id, user_id)`: a participant
/// has at most one ticket per raffle.
#[tokio::test]
async fn a_participant_has_at_most_one_ticket_per_raffle() -> Result<(), Box<dyn std::error::Error>>
{
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;

    seed_ticket(&pool, raffle_id, business_id, user_id).await?;

    let result = sqlx::query(
        r#"
        INSERT INTO tickets (raffle_id, user_id, business_id, code, numbers_count)
        VALUES ($1, $2, $3, $4, 1)
        "#,
    )
    .bind(raffle_id)
    .bind(user_id)
    .bind(business_id)
    .bind(unique_code())
    .execute(&pool)
    .await;

    let err = match result {
        Ok(_) => return Err("a second ticket for the same raffle and user was accepted".into()),
        Err(err) => err,
    };
    // El límite lo cuenta el trigger de la tabla: `max_tickets_per_user` no
    // se puede expresar en un índice único.
    assert!(is_limit_reached(&err), "unexpected error: {err}");

    Ok(())
}

/// The same index, read the other way: a cancelled ticket frees the limit.
///
/// The test depends on the index: without it, the second reservation of the
/// participant would be accepted and the first one would not fail. A test
/// that passes without the guarantee is worse than no test.
#[tokio::test]
async fn a_cancelled_ticket_frees_the_limit_of_the_participant()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;

    let ticket_id = seed_ticket(&pool, raffle_id, business_id, user_id).await?;

    let insert_ticket = r#"
        INSERT INTO tickets (raffle_id, user_id, business_id, code, numbers_count)
        VALUES ($1, $2, $3, $4, 1)
        "#;

    // The trigger of the limit is what rejects this reservation.
    let err = match sqlx::query(insert_ticket)
        .bind(raffle_id)
        .bind(user_id)
        .bind(business_id)
        .bind(unique_code())
        .execute(&pool)
        .await
    {
        Ok(_) => {
            return Err("a second ticket for the same raffle and user was accepted".into());
        }
        Err(err) => err,
    };
    assert!(is_limit_reached(&err), "unexpected error: {err}");

    // The cancelled ticket frees the limit.
    sqlx::query("UPDATE tickets SET status = 'cancelled', cancelled_at = now() WHERE id = $1")
        .bind(ticket_id)
        .execute(&pool)
        .await?;

    let result = sqlx::query(insert_ticket)
        .bind(raffle_id)
        .bind(user_id)
        .bind(business_id)
        .bind(unique_code())
        .execute(&pool)
        .await;

    if let Err(err) = result {
        return Err(format!("the cancelled ticket did not free the limit: {err}").into());
    }

    Ok(())
}

/// The trigger `tickets_business_match_trg`: the ticket belongs to the same
/// business as its raffle.
#[tokio::test]
async fn a_ticket_cannot_belong_to_another_business() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;
    let other_business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;

    let result = sqlx::query(
        r#"
        INSERT INTO tickets (raffle_id, user_id, business_id, code, numbers_count)
        VALUES ($1, $2, $3, $4, 1)
        "#,
    )
    .bind(raffle_id)
    .bind(user_id)
    .bind(other_business_id)
    .bind(unique_code())
    .execute(&pool)
    .await;

    let err = match result {
        Ok(_) => return Err("a ticket of another business was accepted".into()),
        Err(err) => err,
    };
    assert!(
        has_message(&err, "ticket.business_id must match raffle.business_id"),
        "unexpected error: {err}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets WHERE raffle_id = $1")
        .bind(raffle_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0, "the rejected ticket was stored anyway");

    Ok(())
}

/// `audit_log` is append-only: UPDATE and DELETE do not touch the row.
#[tokio::test]
async fn the_audit_log_cannot_be_updated_nor_deleted() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;

    let id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO audit_log (business_id, actor_type, action, entity_type, entity_id)
        VALUES ($1, 'system', 'raffle.published', 'raffle', 'raffle-1')
        RETURNING id
        "#,
    )
    .bind(business_id)
    .fetch_one(&pool)
    .await?;

    let updated = sqlx::query("UPDATE audit_log SET action = 'tampered' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await?;
    assert_eq!(
        updated.rows_affected(),
        0,
        "the update of the audit log was not a no-op"
    );

    let deleted = sqlx::query("DELETE FROM audit_log WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await?;
    assert_eq!(
        deleted.rows_affected(),
        0,
        "the delete of the audit log was not a no-op"
    );

    let action: String = sqlx::query_scalar("SELECT action FROM audit_log WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(action, "raffle.published", "the audit log row was modified");

    Ok(())
}

/// The deletion of a business keeps the audit log.
///
/// The bitácora is append-only, and the `ON DELETE SET NULL` of its foreign
/// keys is an `UPDATE`: the rule that turned the `UPDATE` into a no-op also
/// turned the deletion of a business into an impossible operation
/// ("referential integrity query ... gave unexpected result").
#[tokio::test]
async fn the_deletion_of_a_business_keeps_the_audit_log() -> Result<(), Box<dyn std::error::Error>>
{
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;

    let id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO audit_log (business_id, actor_type, action, entity_type, entity_id)
        VALUES ($1, 'system', 'raffle.published', 'raffle', 'raffle-1')
        RETURNING id
        "#,
    )
    .bind(business_id)
    .fetch_one(&pool)
    .await?;

    sqlx::query("DELETE FROM businesses WHERE id = $1")
        .bind(business_id)
        .execute(&pool)
        .await?;

    let business: Option<Uuid> =
        sqlx::query_scalar("SELECT business_id FROM audit_log WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        business, None,
        "the audit row did not survive the deletion of the business"
    );

    // The row is still there, and it is still append-only.
    let action: String = sqlx::query_scalar("SELECT action FROM audit_log WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(action, "raffle.published", "the audit row was modified");

    Ok(())
}

/// The deletion of a participant keeps the audit log.
///
/// Same mechanism, the other foreign key: the row of the audit log loses the
/// participant that is going away.
#[tokio::test]
async fn the_deletion_of_a_participant_keeps_the_audit_log()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    let user_id = seed_user(&pool).await?;

    let id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO audit_log (actor_user_id, actor_type, action, entity_type, entity_id)
        VALUES ($1, 'user', 'user.profile.updated', 'user', $1::text)
        RETURNING id
        "#,
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await?;

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await?;

    let actor: Option<Uuid> =
        sqlx::query_scalar("SELECT actor_user_id FROM audit_log WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        actor, None,
        "the audit row did not survive the deletion of the participant"
    );

    Ok(())
}

/// The uniqueness of a number holds under concurrency: two participants that
/// ask for the same number at the same time, one of them gets it.
#[tokio::test]
async fn two_concurrent_reservations_of_the_same_number_leave_one_winner()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    let first_user = seed_user(&pool).await?;
    let second_user = seed_user(&pool).await?;

    let first = tokio::spawn({
        let pool = pool.clone();
        let code = unique_code();
        async move {
            with_participant(&pool, first_user, |tx| {
                Box::pin(async move {
                    match reserve_ticket(tx, raffle_id, first_user, &code, &[42], false).await {
                        Ok(_) => Ok::<bool, sqlx::Error>(true),
                        Err(_) => Ok::<bool, sqlx::Error>(false),
                    }
                })
            })
            .await
        }
    });

    let second = tokio::spawn({
        let pool = pool.clone();
        let code = unique_code();
        async move {
            with_participant(&pool, second_user, |tx| {
                Box::pin(async move {
                    match reserve_ticket(tx, raffle_id, second_user, &code, &[42], false).await {
                        Ok(_) => Ok::<bool, sqlx::Error>(true),
                        Err(_) => Ok::<bool, sqlx::Error>(false),
                    }
                })
            })
            .await
        }
    });

    let first_won = first.await??;
    let second_won = second.await??;
    assert!(
        first_won ^ second_won,
        "exactly one of the two reservations had to succeed"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM ticket_numbers WHERE raffle_id = $1")
        .bind(raffle_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1, "the number 42 was reserved more than once");

    Ok(())
}
