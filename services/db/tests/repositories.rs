//! The critical queries of `docs/data-model.md` §6, running against a real
//! database.

mod common;

use common::{seed_business, seed_raffle, seed_user, test_pool, unique_code, with_participant};
use serde_json::json;
use suertudo_db::repositories::{
    DrawSeed, consented_contacts, create_pending_draw, numbers_availability, raffle_metrics,
    raffles_to_draw, reserve_ticket,
};
use uuid::Uuid;

/// §6.1: the numbers available of a raffle.
#[tokio::test]
async fn the_numbers_available_come_from_the_denormalised_counter()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;

    let mut tx = pool.begin().await?;
    let before = numbers_availability(&mut tx, raffle_id).await?;
    assert_eq!(before.number_pool_size, 1000);
    assert_eq!(before.numbers_reserved, 0);
    assert_eq!(before.numbers_available, 1000);
    tx.commit().await?;

    with_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            reserve_ticket(tx, raffle_id, user_id, &unique_code(), &[1, 2, 3], false).await
        })
    })
    .await?;

    let mut tx = pool.begin().await?;
    let after = numbers_availability(&mut tx, raffle_id).await?;
    assert_eq!(after.numbers_reserved, 3);
    assert_eq!(after.numbers_available, 997);
    tx.commit().await?;

    Ok(())
}

/// §6.2: the reservation writes the ticket, its numbers and the counters.
#[tokio::test]
async fn the_reservation_writes_the_ticket_its_numbers_and_the_counters()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;

    let reservation = with_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            reserve_ticket(tx, raffle_id, user_id, &unique_code(), &[10, 11], false).await
        })
    })
    .await?;

    let numbers: Vec<i32> = sqlx::query_scalar(
        "SELECT number FROM ticket_numbers WHERE ticket_id = $1 ORDER BY number",
    )
    .bind(reservation.ticket_id)
    .fetch_all(&pool)
    .await?;
    assert_eq!(numbers, vec![10, 11]);

    let count: i32 = sqlx::query_scalar("SELECT numbers_count FROM tickets WHERE id = $1")
        .bind(reservation.ticket_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 2);

    let counters: (i32, i32) =
        sqlx::query_as("SELECT tickets_count, numbers_reserved FROM raffles WHERE id = $1")
            .bind(raffle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(counters, (1, 2));

    Ok(())
}

/// §6.2: a raffle that is not published does not take reservations.
#[tokio::test]
async fn a_raffle_that_is_not_published_does_not_take_reservations()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "draft").await?;

    let result = with_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            reserve_ticket(tx, raffle_id, user_id, &unique_code(), &[1], false).await
        })
    })
    .await;

    match result {
        Ok(_) => return Err("a draft raffle accepted a reservation".into()),
        Err(err) => {
            let message = err.to_string();
            assert!(
                message.contains("not open for reservations"),
                "unexpected error: {message}"
            );
        }
    }

    Ok(())
}

/// §6.2: the reservation is atomic. A number that is taken rolls the whole
/// transaction back, and no half-reserved ticket survives.
#[tokio::test]
async fn a_taken_number_rolls_the_whole_reservation_back() -> Result<(), Box<dyn std::error::Error>>
{
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;
    let first_user = seed_user(&pool).await?;
    let second_user = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;

    with_participant(&pool, first_user, |tx| {
        Box::pin(async move {
            reserve_ticket(tx, raffle_id, first_user, &unique_code(), &[99], false).await
        })
    })
    .await?;

    let result = with_participant(&pool, second_user, |tx| {
        Box::pin(async move {
            reserve_ticket(tx, raffle_id, second_user, &unique_code(), &[98, 99], false).await
        })
    })
    .await;

    if let Ok(reservation) = result {
        return Err(format!("the reservation {} was accepted", reservation.ticket_id).into());
    }

    let tickets: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets WHERE raffle_id = $1")
        .bind(raffle_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(tickets, 1, "the rejected reservation left a ticket behind");

    let numbers: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ticket_numbers WHERE raffle_id = $1")
            .bind(raffle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(numbers, 1, "the rejected reservation left a number behind");

    Ok(())
}

/// §6.3: the draw of a raffle is created once, no matter how many workers ask.
#[tokio::test]
async fn the_draw_of_a_raffle_is_created_once() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "closed").await?;

    let seed = DrawSeed {
        raffle_id,
        business_id,
        seed_commitment: "commitment".to_string(),
        seed_ciphertext: vec![0u8; 32],
        derivation_spec: json!({"message_template": "suertudo/v1", "hash": "hmac-sha256"}),
        pool_size: 1000,
    };

    let mut tx = pool.begin().await?;
    let first = create_pending_draw(&mut tx, &seed).await?;
    tx.commit().await?;
    assert!(first.is_some(), "the first draw was not created");

    let mut tx = pool.begin().await?;
    let second = create_pending_draw(&mut tx, &seed).await?;
    tx.commit().await?;
    assert!(second.is_none(), "a second draw of the raffle was created");

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM draws WHERE raffle_id = $1")
        .bind(raffle_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1);

    Ok(())
}

/// §6.5: the worker only takes the raffles that are due.
#[tokio::test]
async fn the_worker_only_takes_the_raffles_that_are_due() -> Result<(), Box<dyn std::error::Error>>
{
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;

    let due = seed_raffle(&pool, business_id, "closed").await?;
    sqlx::query("UPDATE raffles SET draws_at = now() - interval '1 minute' WHERE id = $1")
        .bind(due)
        .execute(&pool)
        .await?;

    let later = seed_raffle(&pool, business_id, "closed").await?;
    sqlx::query("UPDATE raffles SET draws_at = now() + interval '1 day' WHERE id = $1")
        .bind(later)
        .execute(&pool)
        .await?;

    let mut tx = pool.begin().await?;
    let due_raffles = raffles_to_draw(&mut tx, 100).await?;
    tx.commit().await?;

    let ids: Vec<Uuid> = due_raffles.iter().map(|raffle| raffle.id).collect();
    assert!(ids.contains(&due), "the raffle that was due was not taken");
    assert!(!ids.contains(&later), "a raffle that is not due was taken");

    Ok(())
}

/// §6.6: the metrics of a raffle.
#[tokio::test]
async fn the_metrics_of_a_raffle_count_the_tickets_and_the_numbers()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;
    let first_user = seed_user(&pool).await?;
    let second_user = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;

    with_participant(&pool, first_user, |tx| {
        Box::pin(async move {
            reserve_ticket(tx, raffle_id, first_user, &unique_code(), &[1, 2], false).await
        })
    })
    .await?;

    with_participant(&pool, second_user, |tx| {
        Box::pin(async move {
            reserve_ticket(tx, raffle_id, second_user, &unique_code(), &[3], false).await
        })
    })
    .await?;

    let mut tx = pool.begin().await?;
    let metrics = raffle_metrics(&mut tx, raffle_id).await?;
    tx.commit().await?;

    assert_eq!(metrics.active_tickets, 2);
    assert_eq!(metrics.winning_tickets, 0);
    assert_eq!(metrics.unique_participants, 2);
    assert_eq!(metrics.numbers_reserved, 3);

    Ok(())
}

/// §6.4: the export of the contacts a business may contact.
///
/// The consent has to predate the ticket, and the last consent of each
/// participant is the one that counts: a revocation after the ticket removes
/// the contact from the export.
#[tokio::test]
async fn the_export_honours_the_last_consent_of_each_participant()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;
    let granted_user = seed_user(&pool).await?;
    let revoked_user = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;

    sqlx::query(
        r#"
        INSERT INTO consent_records (user_id, business_id, scope, granted, source)
        VALUES ($1, $2, 'share_marketing', true, 'signup')
        "#,
    )
    .bind(granted_user)
    .bind(business_id)
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO consent_records (user_id, business_id, scope, granted, source)
        VALUES ($1, $2, 'share_marketing', true, 'signup')
        "#,
    )
    .bind(revoked_user)
    .bind(business_id)
    .execute(&pool)
    .await?;

    with_participant(&pool, granted_user, |tx| {
        Box::pin(async move {
            reserve_ticket(tx, raffle_id, granted_user, &unique_code(), &[1], false).await
        })
    })
    .await?;

    with_participant(&pool, revoked_user, |tx| {
        Box::pin(async move {
            reserve_ticket(tx, raffle_id, revoked_user, &unique_code(), &[2], false).await
        })
    })
    .await?;

    // The second participant revokes the consent after participating.
    sqlx::query(
        r#"
        INSERT INTO consent_records (user_id, business_id, scope, granted, source)
        VALUES ($1, $2, 'share_marketing', false, 'settings')
        "#,
    )
    .bind(revoked_user)
    .bind(business_id)
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    let contacts = consented_contacts(&mut tx, business_id).await?;
    tx.commit().await?;

    let user_ids: Vec<Uuid> = contacts.iter().map(|contact| contact.user_id).collect();
    assert_eq!(
        user_ids,
        vec![granted_user],
        "the revoked consent did not remove the contact from the export"
    );
    assert_eq!(contacts[0].raffle_id, raffle_id);

    Ok(())
}
