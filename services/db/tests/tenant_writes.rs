//! The guarantees that are not about the isolation of a business.
//!
//! Every test here runs with the role of the application: the owner of the
//! tables bypasses Row Level Security by design, and the guarantee has to
//! hold for the role that connects in production.

mod common;

use common::{
    ensure_app_role, has_message, rejected_write, seed_business, seed_raffle, seed_ticket,
    seed_user, test_pool, unique_code, with_app_participant, with_app_role, with_app_system,
};
use uuid::Uuid;

/// A draw cannot name the raffle of another business.
///
/// El draw decide quién gana: es la fila más importante del producto. El
/// RLS comprueba el `business_id` de la fila que se escribe, pero no puede
/// saber a qué sorteo apunta: para eso está la foreign key compuesta.
#[tokio::test]
async fn a_draw_cannot_name_the_raffle_of_another_business()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let other_business_id = seed_business(&pool).await?;
    let other_raffle_id = seed_raffle(&pool, other_business_id, "published").await?;

    let err = rejected_write(&pool, business_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO draws (raffle_id, business_id, seed_commitment, seed_ciphertext,
                                   derivation_spec, pool_size)
                VALUES ($1, $2, 'commitment', $3, '{"hash":"hmac-sha256"}'::jsonb, 1000)
                "#,
            )
            .bind(other_raffle_id)
            .bind(business_id)
            .bind(vec![0u8; 32])
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;
    assert!(
        has_message(&err, "draws_raffle_business_fkey"),
        "unexpected error: {err}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM draws WHERE raffle_id = $1")
        .bind(other_raffle_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0, "the draw was stored anyway");

    Ok(())
}

/// A prize cannot name the raffle of another business.
#[tokio::test]
async fn a_prize_cannot_name_the_raffle_of_another_business()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let other_business_id = seed_business(&pool).await?;
    let other_raffle_id = seed_raffle(&pool, other_business_id, "closed").await?;

    let draw_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO draws (raffle_id, business_id, seed_commitment, seed_ciphertext,
                           derivation_spec, pool_size)
        VALUES ($1, $2, 'commitment', $3, '{"hash":"hmac-sha256"}'::jsonb, 1000)
        RETURNING id
        "#,
    )
    .bind(other_raffle_id)
    .bind(other_business_id)
    .bind(vec![0u8; 32])
    .fetch_one(&pool)
    .await?;

    let err = rejected_write(&pool, business_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO prizes (raffle_id, draw_id, business_id, prize_name)
                VALUES ($1, $2, $3, 'Premio')
                "#,
            )
            .bind(other_raffle_id)
            .bind(draw_id)
            .bind(business_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;
    assert!(
        has_message(&err, "prizes_raffle_business_fkey"),
        "unexpected error: {err}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM prizes WHERE draw_id = $1")
        .bind(draw_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0, "the prize was stored anyway");

    Ok(())
}

/// A business cannot read the outbox of another business.
///
/// El outbox es el patrón del transactional outbox (`ADR-002`): el payload
/// lleva datos del comercio y del participante.
#[tokio::test]
async fn a_business_cannot_read_the_outbox_of_another_business()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let other_business_id = seed_business(&pool).await?;

    for id in [business_id, other_business_id] {
        sqlx::query(
            "INSERT INTO outbox (business_id, topic, payload) VALUES ($1, 'raffle.published', '{}')",
        )
        .bind(id)
        .execute(&pool)
        .await?;
    }

    let businesses: Vec<Option<Uuid>> = with_app_role(&pool, Some(business_id), None, |tx| {
        Box::pin(async move {
            let rows: Vec<Option<Uuid>> = sqlx::query_scalar("SELECT business_id FROM outbox")
                .fetch_all(&mut **tx)
                .await?;
            Ok(rows)
        })
    })
    .await?;
    assert_eq!(
        businesses,
        vec![Some(business_id)],
        "the business read the outbox of another business"
    );

    Ok(())
}

/// The application role cannot list the participants of another business.
///
/// `users` no tenía políticas: una conexión con el rol de la aplicación
/// leía todos los participantes (correo, teléfono, nombre).
#[tokio::test]
async fn the_application_role_cannot_list_the_participants_of_another_business()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let other_business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let other_user_id = seed_user(&pool).await?;

    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    let other_raffle_id = seed_raffle(&pool, other_business_id, "published").await?;
    seed_ticket(&pool, raffle_id, business_id, user_id).await?;
    seed_ticket(&pool, other_raffle_id, other_business_id, other_user_id).await?;

    let participants: Vec<Uuid> = with_app_role(&pool, Some(business_id), None, |tx| {
        Box::pin(async move {
            let rows: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM users")
                .fetch_all(&mut **tx)
                .await?;
            Ok(rows)
        })
    })
    .await?;
    assert_eq!(
        participants,
        vec![user_id],
        "the business read the participants of another business"
    );

    // Sin contexto no se lee a nadie: las políticas fallan cerradas.
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL ROLE suertudo_test_app")
        .execute(&mut *tx)
        .await?;
    let participants: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM users")
        .fetch_all(&mut *tx)
        .await?;
    assert!(
        participants.is_empty(),
        "a transaction without the tenant context read the participants"
    );
    tx.commit().await?;

    Ok(())
}

/// A ticket cannot be reserved in a draft raffle.
///
/// El sorteo es `draft` hasta que el comercio lo publica: un sorteo en
/// borrador no acepta reservas. La aplicación lo valida, pero la garantía
/// tiene que estar en la base.
#[tokio::test]
async fn a_ticket_cannot_be_reserved_in_a_draft_raffle() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let draft_raffle_id = seed_raffle(&pool, business_id, "draft").await?;

    let err = rejected_write(&pool, business_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO tickets (raffle_id, user_id, business_id, code, numbers_count)
                VALUES ($1, $2, $3, $4, 1)
                "#,
            )
            .bind(draft_raffle_id)
            .bind(user_id)
            .bind(business_id)
            .bind(unique_code())
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;
    assert!(
        has_message(&err, "RAFFLE_NOT_OPEN"),
        "unexpected error: {err}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets WHERE raffle_id = $1")
        .bind(draft_raffle_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0, "the ticket of the draft raffle was stored");

    Ok(())
}

/// A participant cancels their own ticket and the numbers come back.
///
/// `FR-PA-07`: el participante puede cancelar su ticket mientras el sorteo
/// esté `published`, y la cancelación libera los números.
#[tokio::test]
async fn a_participant_cancels_their_own_ticket() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    let ticket_id = seed_ticket(&pool, raffle_id, business_id, user_id).await?;

    // The reservation: a number and the counter of the raffle.
    sqlx::query("INSERT INTO ticket_numbers (ticket_id, raffle_id, number) VALUES ($1, $2, 42)")
        .bind(ticket_id)
        .bind(raffle_id)
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE raffles SET numbers_reserved = 1, tickets_count = 1 WHERE id = $1")
        .bind(raffle_id)
        .execute(&pool)
        .await?;

    let updated = with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            let result = sqlx::query(
                "UPDATE tickets SET status = 'cancelled', cancelled_at = now() WHERE id = $1",
            )
            .bind(ticket_id)
            .execute(&mut **tx)
            .await?;
            Ok::<u64, sqlx::Error>(result.rows_affected())
        })
    })
    .await?;
    assert_eq!(updated, 1, "the ticket was not cancelled");

    let status: String = sqlx::query_scalar("SELECT status FROM tickets WHERE id = $1")
        .bind(ticket_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "cancelled", "the ticket was not cancelled");

    // The numbers went back to the pool.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM ticket_numbers WHERE ticket_id = $1")
        .bind(ticket_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        count, 0,
        "the numbers of the cancelled ticket were not released"
    );

    // The counter of the raffle agrees with the pool.
    let reserved: i32 = sqlx::query_scalar("SELECT numbers_reserved FROM raffles WHERE id = $1")
        .bind(raffle_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(reserved, 0, "the counter of the raffle was not released");

    let tickets_count: i32 = sqlx::query_scalar("SELECT tickets_count FROM raffles WHERE id = $1")
        .bind(raffle_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(tickets_count, 0, "the counter of tickets was not released");

    // The number 42 can be reserved again.
    let other_ticket = seed_ticket(&pool, raffle_id, business_id, seed_user(&pool).await?).await?;
    sqlx::query("INSERT INTO ticket_numbers (ticket_id, raffle_id, number) VALUES ($1, $2, 42)")
        .bind(other_ticket)
        .bind(raffle_id)
        .execute(&pool)
        .await?;

    Ok(())
}

/// A participant cannot cancel the ticket of another participant.
#[tokio::test]
async fn a_participant_cannot_cancel_the_ticket_of_another_participant()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let other_user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    let other_ticket = seed_ticket(&pool, raffle_id, business_id, other_user_id).await?;

    let updated = with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            let result = sqlx::query(
                "UPDATE tickets SET status = 'cancelled', cancelled_at = now() WHERE id = $1",
            )
            .bind(other_ticket)
            .execute(&mut **tx)
            .await?;
            Ok::<u64, sqlx::Error>(result.rows_affected())
        })
    })
    .await?;
    assert_eq!(
        updated, 0,
        "the participant cancelled the ticket of another participant"
    );

    let status: String = sqlx::query_scalar("SELECT status FROM tickets WHERE id = $1")
        .bind(other_ticket)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        status, "reserved",
        "the ticket of the other participant was cancelled"
    );

    Ok(())
}

/// A participant cannot cancel a ticket once the raffle is no longer published.
///
/// `FR-PA-07`: la cancelación es "mientras el sorteo esté `published`".
#[tokio::test]
async fn a_participant_cannot_cancel_a_ticket_once_the_raffle_is_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    let ticket_id = seed_ticket(&pool, raffle_id, business_id, user_id).await?;

    sqlx::query("UPDATE raffles SET status = 'closed' WHERE id = $1")
        .bind(raffle_id)
        .execute(&pool)
        .await?;

    let err = match with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                "UPDATE tickets SET status = 'cancelled', cancelled_at = now() WHERE id = $1",
            )
            .bind(ticket_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the participant cancelled a ticket of a closed raffle".into()),
        Err(err) => err,
    };
    assert!(
        has_message(&err, "RAFFLE_CLOSED"),
        "unexpected error: {err}"
    );

    let status: String = sqlx::query_scalar("SELECT status FROM tickets WHERE id = $1")
        .bind(ticket_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "reserved", "the ticket was cancelled anyway");

    Ok(())
}

// ---------------------------------------------------------------------------
// N1: la pertenencia a un comercio
// ---------------------------------------------------------------------------

/// A participant cannot join another business.
///
/// `business_members` es la única fuente de pertenencia: una fila que un
/// participante se escribe con `role = 'owner'` le entrega el commerce al
/// atacante.
///
/// El contexto es el del participante: no hay contexto de comercio que lo
/// habilite.
#[tokio::test]
async fn a_participant_cannot_join_another_business() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;

    let err = match with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO business_members (business_id, user_id, role, status)
                VALUES ($1, $2, 'owner', 'active')
                "#,
            )
            .bind(business_id)
            .bind(user_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the participant joined another business".into()),
        Err(err) => err,
    };
    assert!(
        has_message(&err, "row-level security policy"),
        "unexpected error: {err}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM business_members WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0, "the membership was stored anyway");

    Ok(())
}

// ---------------------------------------------------------------------------
// N2: el ticket y el premio de otro comercio
// ---------------------------------------------------------------------------

/// A participant cannot insert a ticket of another business.
///
/// La rama `user_id = current_user_id()` de la política de `INSERT` de
/// `tickets` autorizaba a insertar un ticket fantasma en el sorteo de otro
/// comercio: el `numbers_count` no tenía números detrás y el contador del
/// sorteo se divergía de la verdad.
#[tokio::test]
async fn a_participant_cannot_insert_a_ticket_of_another_business()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let other_business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let other_raffle_id = seed_raffle(&pool, other_business_id, "published").await?;

    let err = match with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO tickets (raffle_id, user_id, business_id, code, numbers_count)
                VALUES ($1, $2, $3, 'PHANTOM1', 5)
                "#,
            )
            .bind(other_raffle_id)
            .bind(user_id)
            .bind(other_business_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the participant inserted a ticket of another business".into()),
        Err(err) => err,
    };
    assert!(
        has_message(&err, "row-level security policy"),
        "unexpected error: {err}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets WHERE raffle_id = $1")
        .bind(other_raffle_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0, "the phantom ticket was stored");

    Ok(())
}

/// A participant cannot insert a prize of another business.
///
/// El premio con el `claim_code` elegido por el participante ocupaba el lugar
/// del premio real en `prizes_raffle_position_idx`.
#[tokio::test]
async fn a_participant_cannot_insert_a_prize_of_another_business()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let other_business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let other_raffle_id = seed_raffle(&pool, other_business_id, "closed").await?;

    let draw_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO draws (raffle_id, business_id, seed_commitment, seed_ciphertext,
                           derivation_spec, pool_size)
        VALUES ($1, $2, 'commitment', $3, '{"hash":"hmac-sha256"}'::jsonb, 1000)
        RETURNING id
        "#,
    )
    .bind(other_raffle_id)
    .bind(other_business_id)
    .bind(vec![0u8; 32])
    .fetch_one(&pool)
    .await?;

    let err = match with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO prizes (raffle_id, draw_id, business_id, user_id, prize_name, claim_code)
                VALUES ($1, $2, $3, $4, 'UN MILLON', 'CLAIM-123')
                "#,
            )
            .bind(other_raffle_id)
            .bind(draw_id)
            .bind(other_business_id)
            .bind(user_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the participant inserted a prize of another business".into()),
        Err(err) => err,
    };
    assert!(
        has_message(&err, "row-level security policy"),
        "unexpected error: {err}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM prizes WHERE draw_id = $1")
        .bind(draw_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0, "the phantom prize was stored");

    Ok(())
}

// ---------------------------------------------------------------------------
// N3: los contadores del sorteo
// ---------------------------------------------------------------------------

/// The counters of a raffle are not incremented twice.
///
/// `apply_reservation_counters` sumaba y no estaba atada al que llamaba: con
/// el contexto de un participante movía los contadores del sorteo de otro
/// comercio. Ahora recalcula desde la verdad y se niega a actuar por el
/// sorteo de otro.
#[tokio::test]
async fn the_counters_of_a_raffle_are_not_incremented_twice()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let other_business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let owner_id = seed_user(&pool).await?;
    let other_raffle_id = seed_raffle(&pool, other_business_id, "published").await?;
    let other_ticket = seed_ticket(&pool, other_raffle_id, other_business_id, owner_id).await?;

    // El participante no puede recalcular los contadores de un sorteo que no
    // es suyo: la función está atada al dueño del ticket.
    let err = match with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT apply_reservation_counters($1)")
                .bind(other_ticket)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the participant moved the counters of another raffle".into()),
        Err(err) => err,
    };
    assert!(
        has_message(
            &err,
            "the counters of a raffle are recomputed by the platform"
        ),
        "unexpected error: {err}"
    );

    let (reserved, tickets_count): (i32, i32) =
        sqlx::query_as("SELECT numbers_reserved, tickets_count FROM raffles WHERE id = $1")
            .bind(other_raffle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(reserved, 0, "the counter of numbers was moved");
    assert_eq!(tickets_count, 0, "the counter of tickets was moved");

    // Y recalcular dos veces deja lo mismo: el contador sale de la verdad.
    for _ in 0..2 {
        with_app_system(&pool, |tx| {
            Box::pin(async move {
                sqlx::query("SELECT apply_reservation_counters($1)")
                    .bind(other_ticket)
                    .execute(&mut **tx)
                    .await?;
                Ok(())
            })
        })
        .await?;
    }

    let (reserved, tickets_count): (i32, i32) =
        sqlx::query_as("SELECT numbers_reserved, tickets_count FROM raffles WHERE id = $1")
            .bind(other_raffle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(reserved, 0, "the counter of numbers was invented");
    assert_eq!(tickets_count, 1, "the counter of tickets was invented");

    Ok(())
}

// ---------------------------------------------------------------------------
// F1 y F4: los device tokens
// ---------------------------------------------------------------------------

/// A participant does not see the push devices of another participant.
///
/// El token del device es una credencial: con el rol de la aplicación se
/// leían todos y se podían desactivar.
#[tokio::test]
async fn a_participant_does_not_see_the_push_devices_of_another_participant()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let user_id = seed_user(&pool).await?;
    let other_user_id = seed_user(&pool).await?;

    sqlx::query(
        "INSERT INTO push_devices (user_id, platform, token, app_version) VALUES ($1, 'android', $2, '1.0')",
    )
    .bind(user_id)
    .bind(format!("device-{}", Uuid::new_v4()))
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO push_devices (user_id, platform, token, app_version) VALUES ($1, 'android', $2, '1.0')",
    )
    .bind(other_user_id)
    .bind(format!("device-{}", Uuid::new_v4()))
    .execute(&pool)
    .await?;

    let tokens: Vec<String> = with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            let rows: Vec<String> = sqlx::query_scalar("SELECT token FROM push_devices")
                .fetch_all(&mut **tx)
                .await?;
            Ok(rows)
        })
    })
    .await?;
    assert_eq!(
        tokens.len(),
        1,
        "the participant read the devices of another one"
    );

    // Y no puede desactivar el device de otro.
    let updated = with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            let result = sqlx::query("UPDATE push_devices SET active = false WHERE user_id = $1")
                .bind(other_user_id)
                .execute(&mut **tx)
                .await?;
            Ok::<u64, sqlx::Error>(result.rows_affected())
        })
    })
    .await?;
    assert_eq!(
        updated, 0,
        "the participant deactivated the device of another one"
    );

    let active: bool = sqlx::query_scalar("SELECT active FROM push_devices WHERE user_id = $1")
        .bind(other_user_id)
        .fetch_one(&pool)
        .await?;
    assert!(
        active,
        "the device of the other participant was deactivated"
    );

    Ok(())
}

/// The deletion of the account leaves no push device.
///
/// `FR-ID-05`: la baja anonimiza los datos personales. El token del device
/// identifica el teléfono y es una credencial: no sobrevive.
#[tokio::test]
async fn the_deletion_of_the_account_leaves_no_push_device()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let user_id = seed_user(&pool).await?;
    sqlx::query(
        "INSERT INTO push_devices (user_id, platform, token, app_version) VALUES ($1, 'android', $2, '1.0')",
    )
    .bind(user_id)
    .bind(format!("device-{}", Uuid::new_v4()))
    .execute(&pool)
    .await?;

    let deleted: i32 = with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            let count: i32 = sqlx::query_scalar("SELECT anonymise_user_devices($1)")
                .bind(user_id)
                .fetch_one(&mut **tx)
                .await?;
            Ok(count)
        })
    })
    .await?;
    assert_eq!(deleted, 1, "the device token was not removed");

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM push_devices WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        count, 0,
        "the device token survived the deletion of the account"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// F2: la familia de refresh
// ---------------------------------------------------------------------------

/// A successor cannot enter a revoked family of another participant.
///
/// El trigger que vigila la familia leía la fila de `token_families` con un
/// `SELECT` filtrado por el RLS: no veía la familia revocada y dejaba
/// entrar el sucesor.
#[tokio::test]
async fn a_successor_cannot_enter_a_revoked_family_of_another_participant()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let user_id = seed_user(&pool).await?;

    // The family of the other participant, revoked.
    let family_id: Uuid = with_app_participant(&pool, owner_id, |tx| {
        Box::pin(async move {
            let id: Uuid =
                sqlx::query_scalar("INSERT INTO token_families (user_id) VALUES ($1) RETURNING id")
                    .bind(owner_id)
                    .fetch_one(&mut **tx)
                    .await?;
            Ok(id)
        })
    })
    .await?;

    with_app_participant(&pool, owner_id, |tx| {
        Box::pin(async move {
            sqlx::query("UPDATE token_families SET revoked_at = now() WHERE id = $1")
                .bind(family_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    // The successor of the other participant does not enter.
    let err = match with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO auth_tokens (user_id, email, kind, token_hash, family_id, expires_at)
                VALUES ($1, 'family@example.test', 'refresh', $2, $3, now() + interval '1 day')
                "#,
            )
            .bind(user_id)
            .bind(Uuid::new_v4().as_bytes().to_vec())
            .bind(family_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the successor entered the revoked family of another one".into()),
        Err(err) => err,
    };
    assert!(
        has_message(&err, "the refresh token family is revoked"),
        "unexpected error: {err}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_tokens WHERE family_id = $1")
        .bind(family_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0, "the successor was stored anyway");

    Ok(())
}

// ---------------------------------------------------------------------------
// F3: el ticket cancelado
// ---------------------------------------------------------------------------

/// A participant cannot change a cancelled ticket.
///
/// La rama de la política permite `status = 'cancelled'`, y la política no
/// distinguía "el participante cancela" de "el participante modifica un
/// ticket ya cancelado": con el ticket cancelado, la fila pasaba el
/// `WITH CHECK` con cualquier `numbers_count`.
#[tokio::test]
async fn a_participant_cannot_change_a_cancelled_ticket() -> Result<(), Box<dyn std::error::Error>>
{
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    let ticket_id = seed_ticket(&pool, raffle_id, business_id, user_id).await?;

    with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                "UPDATE tickets SET status = 'cancelled', cancelled_at = now() WHERE id = $1",
            )
            .bind(ticket_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;

    let err = match with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query("UPDATE tickets SET numbers_count = 99 WHERE id = $1")
                .bind(ticket_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the participant changed a cancelled ticket".into()),
        Err(err) => err,
    };
    assert!(
        has_message(&err, "a cancelled ticket cannot be changed"),
        "unexpected error: {err}"
    );

    let numbers_count: i32 = sqlx::query_scalar("SELECT numbers_count FROM tickets WHERE id = $1")
        .bind(ticket_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(numbers_count, 1, "the cancelled ticket was changed");

    Ok(())
}

// ---------------------------------------------------------------------------
// F5: el límite de tickets por participante
// ---------------------------------------------------------------------------

/// The raffle honours `max_tickets_per_user`.
///
/// El índice parcial único fijaba un ticket por `(raffle, user)`: un sorteo
/// con `max_tickets_per_user = 2` le negaba al participante su segundo
/// ticket. El límite lo cuenta un trigger, que compara los tickets vivos
/// del participante contra la columna.
#[tokio::test]
async fn the_raffle_honours_max_tickets_per_user() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let user_id = seed_user(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;
    sqlx::query("UPDATE raffles SET max_tickets_per_user = 2 WHERE id = $1")
        .bind(raffle_id)
        .execute(&pool)
        .await?;

    // The first two tickets are accepted.
    for number in [11, 12] {
        let ticket_id: Uuid = with_app_participant(&pool, user_id, |tx| {
            Box::pin(async move {
                let id: Uuid = sqlx::query_scalar("SELECT reserve_ticket($1, $2, $3, $4, $5)")
                    .bind(raffle_id)
                    .bind(user_id)
                    .bind(unique_code())
                    .bind(vec![number])
                    .bind(false)
                    .fetch_one(&mut **tx)
                    .await?;
                Ok(id)
            })
        })
        .await?;
        assert!(!ticket_id.is_nil(), "the ticket was not reserved");
    }

    // The third one is refused with the refusal of the limit.
    let err = match with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT reserve_ticket($1, $2, $3, $4, $5)")
                .bind(raffle_id)
                .bind(user_id)
                .bind(unique_code())
                .bind(vec![13])
                .bind(false)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the participant went over the limit of tickets".into()),
        Err(err) => err,
    };

    assert!(
        has_message(&err, "TICKET_LIMIT_REACHED"),
        "unexpected error: {err}"
    );
    let sqlstate = err
        .as_database_error()
        .and_then(|db| db.code())
        .map(|code| code.into_owned());
    assert_eq!(
        sqlstate.as_deref(),
        Some("PT001"),
        "the refusal of the limit is not distinguishable"
    );

    let tickets: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tickets WHERE raffle_id = $1 AND user_id = $2")
            .bind(raffle_id)
            .bind(user_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(tickets, 2, "the refused reservation left a ticket behind");

    Ok(())
}

// ---------------------------------------------------------------------------
// La auditoría entera
// ---------------------------------------------------------------------------

/// The write policies of a tenant table are bound to the tenant.
///
/// La regla: una tabla que lleva `business_id` sólo la escribe el comercio
/// dueño de la fila (o la plataforma). Una rama `user_id =
/// current_user_id()` sin atar al comercio autoriza a escribir en las filas
/// de otro tenant: es la clase de defecto detrás de N1 y N2.
///
/// El test lee el catálogo: falla cuando una política se edita mal, sin
/// depender de que alguien se acuerde de escribir el test del defecto.
#[tokio::test]
async fn the_write_policies_of_a_tenant_table_are_bound_to_the_tenant()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;

    // The tables that carry a `business_id` (and `businesses`, which is its
    // own tenant).
    let tables: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT table_name FROM information_schema.columns
         WHERE table_schema = 'public' AND column_name = 'business_id'
        UNION
        SELECT 'businesses'
        "#,
    )
    .fetch_all(&pool)
    .await?;

    // The exceptions, one by one: the participant writes its own ticket (the
    // cancellation) and its own notification (the read receipt).
    let exceptions = [
        ("tickets", "tickets_update"),
        ("notifications", "notifications_update"),
    ];

    let policies: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT c.relname,
               p.polname,
               pg_get_expr(p.polqual, p.polrelid),
               pg_get_expr(p.polwithcheck, p.polrelid)
          FROM pg_policy p
          JOIN pg_class c ON c.oid = p.polrelid
         WHERE p.polcmd IN ('a', 'w', 'd', '*')
        "#,
    )
    .fetch_all(&pool)
    .await?;

    let mut unbound: Vec<String> = Vec::new();

    for (table, policy, qual, with_check) in &policies {
        if !tables.contains(table) {
            continue;
        }
        if exceptions.contains(&(table.as_str(), policy.as_str())) {
            continue;
        }

        for expression in [qual, with_check].into_iter().flatten() {
            if expression.contains("current_user_id()") {
                unbound.push(format!("{table}.{policy}: {expression}"));
            }
        }
    }

    assert!(
        unbound.is_empty(),
        "the write policies of a tenant table have an unbound user_id branch: {}",
        unbound.join(" | ")
    );

    Ok(())
}
