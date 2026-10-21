//! Row Level Security: the second barrier of the isolation between businesses.
//!
//! The policies of `docs/data-model.md` §7.3 have to fail **closed**: a
//! connection without the tenant context reads nothing. The tests run as a
//! role that is not the owner of the tables, because the owner of a table
//! bypasses Row Level Security by design.

mod common;

use common::{
    ensure_app_role, has_message, rejected_write, seed_business, seed_raffle, seed_ticket,
    seed_user, test_pool, unique_slug, with_app_role,
};
use uuid::Uuid;

/// A connection without the tenant context sees no tickets.
#[tokio::test]
async fn a_transaction_without_the_tenant_context_sees_no_tickets()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;
    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    seed_ticket(&pool, raffle_id, business_id, user_id).await?;

    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL ROLE suertudo_test_app")
        .execute(&mut *tx)
        .await?;

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets")
        .fetch_one(&mut *tx)
        .await?;
    assert_eq!(
        count, 0,
        "a transaction without the tenant context read the tickets of a business"
    );

    tx.commit().await?;
    Ok(())
}

/// A business sees its own tickets and nobody else's.
#[tokio::test]
async fn a_business_sees_only_its_own_tickets() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let other_business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    let other_raffle_id = seed_raffle(&pool, other_business_id, "published").await?;
    let own_ticket = seed_ticket(&pool, raffle_id, business_id, user_id).await?;
    seed_ticket(&pool, other_raffle_id, other_business_id, user_id).await?;

    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL ROLE suertudo_test_app")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.business_id', $1, true)")
        .bind(business_id.to_string())
        .execute(&mut *tx)
        .await?;

    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM tickets")
        .fetch_all(&mut *tx)
        .await?;
    assert_eq!(
        ids,
        vec![own_ticket],
        "the business read the tickets of another business"
    );

    tx.commit().await?;
    Ok(())
}

/// A participant sees their own tickets and nobody else's.
#[tokio::test]
async fn a_participant_sees_only_their_own_tickets() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let other_user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    let own_ticket = seed_ticket(&pool, raffle_id, business_id, user_id).await?;
    seed_ticket(&pool, raffle_id, business_id, other_user_id).await?;

    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL ROLE suertudo_test_app")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;

    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM tickets")
        .fetch_all(&mut *tx)
        .await?;
    assert_eq!(
        ids,
        vec![own_ticket],
        "the participant read the tickets of another participant"
    );

    tx.commit().await?;
    Ok(())
}

/// The numbers of a ticket inherit the isolation of the ticket.
#[tokio::test]
async fn the_numbers_of_another_business_are_not_readable() -> Result<(), Box<dyn std::error::Error>>
{
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let other_business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    let other_raffle_id = seed_raffle(&pool, other_business_id, "published").await?;

    let own_ticket = seed_ticket(&pool, raffle_id, business_id, user_id).await?;
    let other_ticket = seed_ticket(&pool, other_raffle_id, other_business_id, user_id).await?;
    sqlx::query("INSERT INTO ticket_numbers (ticket_id, raffle_id, number) VALUES ($1, $2, 1)")
        .bind(own_ticket)
        .bind(raffle_id)
        .execute(&pool)
        .await?;
    sqlx::query("INSERT INTO ticket_numbers (ticket_id, raffle_id, number) VALUES ($1, $2, 1)")
        .bind(other_ticket)
        .bind(other_raffle_id)
        .execute(&pool)
        .await?;

    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL ROLE suertudo_test_app")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.business_id', $1, true)")
        .bind(business_id.to_string())
        .execute(&mut *tx)
        .await?;

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM ticket_numbers")
        .fetch_one(&mut *tx)
        .await?;
    assert_eq!(
        count, 1,
        "the business read the numbers of another business"
    );

    tx.commit().await?;
    Ok(())
}

/// A published raffle is public: the feed and the share link have to work
/// without a tenant context.
#[tokio::test]
async fn a_published_raffle_is_public() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let published = seed_raffle(&pool, business_id, "published").await?;
    let draft = seed_raffle(&pool, business_id, "draft").await?;

    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL ROLE suertudo_test_app")
        .execute(&mut *tx)
        .await?;

    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM raffles")
        .fetch_all(&mut *tx)
        .await?;
    assert!(
        ids.contains(&published),
        "a published raffle has to be readable without a tenant context"
    );
    assert!(
        !ids.contains(&draft),
        "a draft raffle leaked without a tenant context"
    );

    tx.commit().await?;
    Ok(())
}

/// The tenant context of a transaction does not survive it: the next
/// transaction on the same connection starts without a context.
#[tokio::test]
async fn the_tenant_context_does_not_leak_between_transactions()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    seed_ticket(&pool, raffle_id, business_id, user_id).await?;

    {
        let mut tx = pool.begin().await?;
        sqlx::query("SET LOCAL ROLE suertudo_test_app")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT set_config('app.business_id', $1, true)")
            .bind(business_id.to_string())
            .execute(&mut *tx)
            .await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets")
            .fetch_one(&mut *tx)
            .await?;
        assert_eq!(count, 1, "the business did not read its own tickets");
        tx.commit().await?;
    }

    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL ROLE suertudo_test_app")
        .execute(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets")
        .fetch_one(&mut *tx)
        .await?;
    assert_eq!(
        count, 0,
        "the tenant context of the previous transaction leaked into this one"
    );
    tx.commit().await?;

    Ok(())
}

/// The helper of the pool sets the context the policies read.
#[tokio::test]
async fn with_tenant_sets_the_context_of_the_policies() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    let business_id = seed_business(&pool).await?;

    let context: Option<String> = suertudo_db::with_tenant(
        &pool,
        suertudo_db::TenantContext::business(business_id),
        |tx| {
            Box::pin(async move {
                let context: Option<String> =
                    sqlx::query_scalar("SELECT current_setting('app.business_id', true)")
                        .fetch_one(&mut **tx)
                        .await?;
                Ok::<Option<String>, sqlx::Error>(context)
            })
        },
    )
    .await?;
    assert_eq!(
        context,
        Some(business_id.to_string()),
        "the tenant context was not applied"
    );

    // Without the helper there is no context at all, and the policies of the
    // database fail closed.
    let mut tx = pool.begin().await?;
    let context: Option<String> =
        sqlx::query_scalar("SELECT current_setting('app.business_id', true)")
            .fetch_one(&mut *tx)
            .await?;
    assert_eq!(context, None, "a context appeared out of nowhere");
    tx.commit().await?;

    Ok(())
}

/// A tenant cannot write the rows of another business.
///
/// The policies only had `USING`, and a policy without `WITH CHECK` uses the
/// `USING` for the `INSERT` and the `UPDATE`: the public read branches (a
/// published raffle, a revealed draw) were write permissions. With the
/// context of one business, another business's rows could be inserted and
/// modified.
///
/// The test runs with the role of the application. The owner of the tables
/// bypasses Row Level Security by design, so a test that connects as the
/// owner would pass without the policies.
#[tokio::test]
async fn a_tenant_cannot_write_the_rows_of_another_business()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let other_business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let other_raffle_id = seed_raffle(&pool, other_business_id, "published").await?;
    let other_raffle_id_two = seed_raffle(&pool, other_business_id, "published").await?;

    // A revealed draw of another business: the public branch of the read
    // policy used to authorize the write.
    sqlx::query(
        r#"
        INSERT INTO draws (raffle_id, business_id, seed_commitment, seed_ciphertext,
                           derivation_spec, seed_revealed, winning_numbers, pool_size)
        VALUES ($1, $2, 'commitment', $3, '{"hash":"hmac-sha256"}'::jsonb, true, '{7}', 1000)
        "#,
    )
    .bind(other_raffle_id)
    .bind(other_business_id)
    .bind(vec![0u8; 32])
    .execute(&pool)
    .await?;

    // A raffle of another business, published: the public branch of the read
    // policy used to authorize the write.
    let err = rejected_write(&pool, business_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO raffles (business_id, slug, title, prize_name, status,
                                     number_pool_size, numbers_per_ticket_min,
                                     numbers_per_ticket_max, max_tickets_per_user)
                VALUES ($1, $2, 'Robado', 'Premio', 'published', 1000, 1, 5, 1)
                "#,
            )
            .bind(other_business_id)
            .bind(unique_slug())
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;
    assert!(
        has_message(&err, "row-level security policy"),
        "unexpected error: {err}"
    );

    // A draw of another business, revealed: the public branch of the read
    // policy used to authorize the write, with a forged winner.
    let err = rejected_write(&pool, business_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO draws (raffle_id, business_id, seed_commitment, seed_ciphertext,
                                   derivation_spec, seed_revealed, winning_numbers, pool_size)
                VALUES ($1, $2, 'commitment', $3, '{"hash":"hmac-sha256"}'::jsonb, true, '{7}', 1000)
                "#,
            )
            .bind(other_raffle_id_two)
            .bind(other_business_id)
            .bind(vec![0u8; 32])
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;
    assert!(
        has_message(&err, "row-level security policy"),
        "unexpected error: {err}"
    );

    // A ticket of the raffle of another business.
    let err = rejected_write(&pool, business_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO tickets (raffle_id, user_id, business_id, code, numbers_count)
                VALUES ($1, $2, $3, $4, 1)
                "#,
            )
            .bind(other_raffle_id)
            .bind(user_id)
            .bind(other_business_id)
            .bind(common::unique_code())
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;
    assert!(
        has_message(&err, "row-level security policy"),
        "unexpected error: {err}"
    );

    // The modification of the raffle of another business. The row is visible
    // (it is published): the write is what has to be refused.
    let err = rejected_write(&pool, business_id, |tx| {
        Box::pin(async move {
            sqlx::query("UPDATE raffles SET title = 'Robado' WHERE id = $1")
                .bind(other_raffle_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;
    assert!(
        has_message(&err, "row-level security policy"),
        "unexpected error: {err}"
    );

    // The modification of the revealed draw of another business.
    let err = rejected_write(&pool, business_id, |tx| {
        Box::pin(async move {
            sqlx::query("UPDATE draws SET winning_numbers = '{7}' WHERE raffle_id = $1")
                .bind(other_raffle_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;
    assert!(
        has_message(&err, "row-level security policy"),
        "unexpected error: {err}"
    );

    // Nothing was written.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM raffles WHERE business_id = $1")
        .bind(other_business_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 2, "the raffle of another business was written");

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM draws WHERE business_id = $1")
        .bind(other_business_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1, "the draw of another business was written");

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets WHERE business_id = $1")
        .bind(other_business_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0, "the ticket of another business was written");

    Ok(())
}

/// The trigger `tickets_business_match_trg` sees the raffle of another tenant.
///
/// The trigger compares `tickets.business_id` with the `business_id` of the
/// raffle, and it used to read it with a `SELECT` that the Row Level Security
/// of the caller filters: with the context of a business, the raffle of
/// another business returned no row, the comparison was `NULL` and the
/// trigger raised nothing. A ticket of another raffle entered the database.
///
/// The test runs with the role of the application: the owner of the tables
/// bypasses the policies, and the trigger was untested against them.
#[tokio::test]
async fn the_business_match_trigger_sees_the_raffle_of_another_tenant()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let other_business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    // A draft: a draft is not public, so the trigger only sees it if it
    // reads the row without the policies of the caller.
    let other_raffle_id = seed_raffle(&pool, other_business_id, "draft").await?;

    let err = rejected_write(&pool, business_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO tickets (raffle_id, user_id, business_id, code, numbers_count)
                VALUES ($1, $2, $3, $4, 1)
                "#,
            )
            .bind(other_raffle_id)
            .bind(user_id)
            .bind(business_id)
            .bind(common::unique_code())
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;
    assert!(
        has_message(&err, "ticket.business_id must match raffle.business_id"),
        "unexpected error: {err}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets WHERE raffle_id = $1")
        .bind(other_raffle_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0, "the rejected ticket was stored anyway");

    Ok(())
}

/// The account deletion can anonymise a ticket.
///
/// `DELETE /v1/me` writes `tickets.user_id = NULL`: the ticket survives the
/// participant without an owner.
///
/// It cannot be a plain `UPDATE` with the context of the participant: the
/// anonymised row is invisible for the participant that anonymised it, and
/// Postgres refuses an `UPDATE` whose new row is not visible for who writes
/// it. The guarantee is in the base: `anonymise_user_tickets`.
///
/// The test runs with the role of the application.
#[tokio::test]
async fn the_account_deletion_can_anonymise_a_ticket() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    let ticket_id = seed_ticket(&pool, raffle_id, business_id, user_id).await?;

    with_app_role(&pool, None, Some(user_id), |tx| {
        Box::pin(async move {
            sqlx::query("SELECT anonymise_user_tickets($1)")
                .bind(user_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    let owner: Option<Uuid> = sqlx::query_scalar("SELECT user_id FROM tickets WHERE id = $1")
        .bind(ticket_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(owner, None, "the ticket still has an owner");

    Ok(())
}

/// The anonymisation of the tickets of another participant is refused.
///
/// The function is `SECURITY DEFINER`: without the check of the context,
/// any connection with the role of the application could take the ownership
/// of the tickets of anybody.
#[tokio::test]
async fn the_anonymisation_of_another_participant_is_refused()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let other_user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    seed_ticket(&pool, raffle_id, business_id, other_user_id).await?;

    let err = match with_app_role(&pool, None, Some(user_id), |tx| {
        Box::pin(async move {
            let count: i32 = sqlx::query_scalar("SELECT anonymise_user_tickets($1)")
                .bind(other_user_id)
                .fetch_one(&mut **tx)
                .await?;
            Ok(count)
        })
    })
    .await
    {
        Ok(_) => return Err("a participant anonymised the tickets of another one".into()),
        Err(err) => err,
    };
    assert!(
        has_message(
            &err,
            "the tickets of another participant cannot be anonymised"
        ),
        "unexpected error: {err}"
    );

    let owner: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM tickets WHERE raffle_id = $1")
            .bind(raffle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        owner,
        Some(other_user_id),
        "the ticket of the other participant was anonymised"
    );

    Ok(())
}

/// The anonymisation cannot be used to change the ticket.
///
/// The commerce may anonymise a ticket of its own raffle, but the
/// anonymisation only takes the ownership away: the rest of the row is the
/// record of what happened in the raffle.
#[tokio::test]
async fn the_anonymisation_cannot_change_the_ticket() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    seed_ticket(&pool, raffle_id, business_id, user_id).await?;

    let err = match with_app_role(&pool, Some(business_id), None, |tx| {
        Box::pin(async move {
            sqlx::query("UPDATE tickets SET user_id = NULL, status = 'winner' WHERE user_id = $1")
                .bind(user_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(()) => {
            return Err("the anonymisation changed the status of the ticket".into());
        }
        Err(err) => err,
    };
    assert!(
        has_message(&err, "anonymisation"),
        "unexpected error: {err}"
    );

    // The participant cannot change the status of their own ticket either:
    // the only change they are allowed is the cancellation.
    let err = match with_app_role(&pool, None, Some(user_id), |tx| {
        Box::pin(async move {
            sqlx::query("UPDATE tickets SET status = 'winner' WHERE user_id = $1")
                .bind(user_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => {
            return Err("the participant changed the status of their own ticket".into());
        }
        Err(err) => err,
    };
    assert!(
        has_message(&err, "can only cancel"),
        "unexpected error: {err}"
    );

    Ok(())
}

/// The audit log is isolated by business.
///
/// A connection with the role of the application reads the diff of audit of
/// every business: the bitácora had no policies at all.
#[tokio::test]
async fn the_audit_log_is_isolated_by_business() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let other_business_id = seed_business(&pool).await?;

    for id in [business_id, other_business_id] {
        sqlx::query(
            r#"
            INSERT INTO audit_log (business_id, actor_type, action, entity_type, entity_id)
            VALUES ($1, 'system', 'raffle.published', 'raffle', $1::text)
            "#,
        )
        .bind(id)
        .execute(&pool)
        .await?;
    }

    // A row without a business: an action of the platform.
    sqlx::query(
        r#"
        INSERT INTO audit_log (business_id, actor_type, action, entity_type, entity_id)
        VALUES (NULL, 'system', 'user.deleted', 'user', 'someone')
        "#,
    )
    .execute(&pool)
    .await?;

    let businesses: Vec<Option<Uuid>> = with_app_role(&pool, Some(business_id), None, |tx| {
        Box::pin(async move {
            let rows: Vec<Option<Uuid>> = sqlx::query_scalar("SELECT business_id FROM audit_log")
                .fetch_all(&mut **tx)
                .await?;
            Ok(rows)
        })
    })
    .await?;
    assert_eq!(
        businesses,
        vec![Some(business_id)],
        "the business read the audit log of another business"
    );

    Ok(())
}

/// A business that was deleted logically is not visible to itself.
///
/// The policy of `businesses` did not have the `deleted_at IS NULL` filter
/// that the policy of `raffles` had: a deleted row stayed alive for its own
/// commerce. The two policies filter the same way now.
#[tokio::test]
async fn a_deleted_business_is_not_visible_to_itself() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    sqlx::query("UPDATE businesses SET deleted_at = now() WHERE id = $1")
        .bind(business_id)
        .execute(&pool)
        .await?;

    let count: i64 = with_app_role(&pool, Some(business_id), None, |tx| {
        Box::pin(async move {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM businesses")
                .fetch_one(&mut **tx)
                .await?;
            Ok(count)
        })
    })
    .await?;
    assert_eq!(
        count, 0,
        "the deleted business was still visible to its own commerce"
    );

    Ok(())
}
