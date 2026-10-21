//! The refusals of the schema: what the database says, and to whom.
//!
//! Every test here runs with the role of the application: the owner of the
//! tables bypasses Row Level Security by design, and the guarantee has to
//! hold for the role that connects in production.
//!
//! A refusal that the worker cannot translate is a refusal that reaches the
//! client as a `500`. The SQLSTATE is part of the contract
//! (`docs/data-model.md` §3.25), so the tests check it and not only the
//! message.

mod common;

use common::{
    ensure_app_role, seed_business, seed_raffle, seed_user, test_pool, with_app_participant,
    with_app_role, with_app_system,
};
use uuid::Uuid;

/// The SQLSTATEs the domain maps to a code of the catalogue
/// (`docs/data-model.md` §3.25).
const MAPPED_SQLSTATES: [&str; 10] = [
    "PT001", "PT002", "PT003", "PT004", "PT005", "PT006", "42501", "55000", "23503", "23505",
];

/// The SQLSTATE of an error, when it comes from the database.
fn sqlstate(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|db| db.code())
        .map(|code| code.into_owned())
}

/// The SQLSTATE a `RAISE EXCEPTION` declares, when it declares one.
fn errcode_of(statement: &str) -> Option<String> {
    let rest = statement.split("ERRCODE").nth(1)?.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('\'')?;
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

/// Every refusal of the schema has a SQLSTATE the worker can translate.
///
/// `RAISE EXCEPTION` without `ERRCODE` raises the default `P0001`, and the
/// identity worker does not know it: the refusal arrives as a `500`
/// `INTERNAL_ERROR` instead of a `403 RBAC_FORBIDDEN`. The class does not come
/// back when the test reads the schema: every `RAISE` of every live function
/// has to declare a SQLSTATE of the catalogue.
#[tokio::test]
async fn every_refusal_of_the_schema_has_a_mapped_sqlstate()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;

    let raisers: Vec<(String, String)> = sqlx::query_as(
        r#"
        SELECT p.proname, p.prosrc
          FROM pg_proc p
          JOIN pg_namespace n ON n.oid = p.pronamespace
         WHERE n.nspname = 'public'
           AND p.prosrc LIKE '%RAISE EXCEPTION%'
        "#,
    )
    .fetch_all(&pool)
    .await?;

    let mut unmapped: Vec<String> = Vec::new();

    for (name, source) in &raisers {
        // The body of a function is a sequence of statements: every `RAISE`
        // ends with a semicolon.
        for statement in source.split(';') {
            if !statement.contains("RAISE EXCEPTION") {
                continue;
            }

            match errcode_of(statement) {
                Some(code) if MAPPED_SQLSTATES.contains(&code.as_str()) => {}
                Some(code) => unmapped.push(format!("{name}: SQLSTATE {code}")),
                None => unmapped.push(format!("{name}: {statement}")),
            }
        }
    }

    assert!(
        unmapped.is_empty(),
        "the refusals of the schema that the worker cannot translate: {}",
        unmapped.join(" | ")
    );

    Ok(())
}

/// A participant cannot clear the moderation of their own account.
///
/// `FR-PA-10` puts an account in `pending_review` and the account cannot
/// reserve numbers while it is there. The moderation is the platform's: the
/// participant cannot move their own `status`.
#[tokio::test]
async fn a_participant_cannot_change_the_status_of_their_own_account()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let user_id = seed_user(&pool).await?;

    // The platform suspends the account.
    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await?;

    let err = match with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query("UPDATE users SET status = 'active' WHERE id = $1")
                .bind(user_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the participant cleared the moderation of their own account".into()),
        Err(err) => err,
    };

    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "unexpected error: {err}"
    );

    let status: String = sqlx::query_scalar("SELECT status FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        status, "suspended",
        "the participant cleared the moderation"
    );

    Ok(())
}

/// A participant still updates its own profile.
///
/// The trigger freezes the columns of the platform, no el perfil: el nombre,
/// el teléfono, el idioma, la zona horaria y las preferencias son del
/// participante.
#[tokio::test]
async fn a_participant_can_update_their_own_profile() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let user_id = seed_user(&pool).await?;

    let updated = with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            let result = sqlx::query(
                r#"
                UPDATE users
                   SET display_name = 'Otro nombre',
                       locale = 'es-MX',
                       marketing_opt_in = true,
                       updated_at = now()
                 WHERE id = $1 AND deleted_at IS NULL
                "#,
            )
            .bind(user_id)
            .execute(&mut **tx)
            .await?;
            Ok::<u64, sqlx::Error>(result.rows_affected())
        })
    })
    .await?;
    assert_eq!(
        updated, 1,
        "the participant could not update its own profile"
    );

    let (display_name, marketing): (String, bool) =
        sqlx::query_as("SELECT display_name, marketing_opt_in FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(display_name, "Otro nombre");
    assert!(marketing);

    Ok(())
}

/// A participant still deletes its own account.
///
/// `FR-ID-05`: la baja anonimiza la fila. Es la única escritura de las
/// columnas de la plataforma que el producto le concede a un participante,
/// y tiene que seguir funcionando.
#[tokio::test]
async fn a_participant_can_delete_their_own_account() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let user_id = seed_user(&pool).await?;

    let updated = with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            let result = sqlx::query(
                r#"
                UPDATE users
                   SET email = 'deleted+' || id::text || '@suertudo.invalid',
                       display_name = 'Deleted user',
                       phone_e164 = NULL,
                       marketing_opt_in = false,
                       status = 'deleted',
                       deleted_at = now(),
                       updated_at = now()
                 WHERE id = $1 AND deleted_at IS NULL
                "#,
            )
            .bind(user_id)
            .execute(&mut **tx)
            .await?;
            Ok::<u64, sqlx::Error>(result.rows_affected())
        })
    })
    .await?;
    assert_eq!(
        updated, 1,
        "the participant could not delete its own account"
    );

    let (status, deleted): (String, bool) =
        sqlx::query_as("SELECT status, deleted_at IS NOT NULL FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(status, "deleted");
    assert!(deleted);

    Ok(())
}

/// The platform still moderates an account.
///
/// El contexto de sistema es la plataforma: la moderación (`FR-PA-10`) y la
/// baja las hace ella.
#[tokio::test]
async fn the_platform_can_moderate_an_account() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let user_id = seed_user(&pool).await?;

    let updated = with_app_system(&pool, |tx| {
        Box::pin(async move {
            let result = sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
                .bind(user_id)
                .execute(&mut **tx)
                .await?;
            Ok::<u64, sqlx::Error>(result.rows_affected())
        })
    })
    .await?;
    assert_eq!(updated, 1, "the platform could not moderate the account");

    let status: String = sqlx::query_scalar("SELECT status FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "suspended");

    Ok(())
}

/// A participant cannot resurrect a deleted account.
///
/// The deletion is a soft delete: the row stays, with the `status` and the
/// `deleted_at` of a deleted account. A participant that can write those two
/// columns brings its own account back, with the moderation it had.
#[tokio::test]
async fn a_participant_cannot_resurrect_a_deleted_account() -> Result<(), Box<dyn std::error::Error>>
{
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let user_id = seed_user(&pool).await?;

    // The account is deleted.
    sqlx::query(
        r#"
        UPDATE users
           SET status = 'deleted',
               deleted_at = now(),
               email = 'deleted+' || id::text || '@suertudo.invalid'
         WHERE id = $1
        "#,
    )
    .bind(user_id)
    .execute(&pool)
    .await?;

    let err = match with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query(
                r#"
                UPDATE users
                   SET deleted_at = NULL,
                       email = 'back@example.test',
                       status = 'active'
                 WHERE id = $1
                "#,
            )
            .bind(user_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the participant resurrected a deleted account".into()),
        Err(err) => err,
    };

    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "unexpected error: {err}"
    );

    let (status, deleted, email): (String, bool, String) = sqlx::query_as(
        "SELECT status, deleted_at IS NOT NULL, email::text FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(status, "deleted", "the deleted account is active again");
    assert!(deleted, "the deleted account lost its deletion date");
    assert!(
        email.ends_with("@suertudo.invalid"),
        "the email of the deleted account was changed: {email}"
    );

    Ok(())
}

/// A participant cannot take the device token of another participant.
///
/// The token of a device is a credencial: the platform sends the notification
/// to it. The registration takes the token from the body of the request, and
/// the function is `SECURITY DEFINER`, so the policies of `push_devices`
/// cannot stop the theft: the conflict update has to check the owner.
#[tokio::test]
async fn a_participant_cannot_take_over_the_device_token_of_another_participant()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let attacker_id = seed_user(&pool).await?;
    let token = format!("tok-{}", Uuid::new_v4());
    let stolen_token = token.clone();
    let owner_token = token.clone();

    // The owner registers the device.
    let device_id: Uuid = with_app_participant(&pool, owner_id, |tx| {
        Box::pin(async move {
            let id: Uuid =
                sqlx::query_scalar("SELECT register_push_device($1, 'android', $2, '1.0')")
                    .bind(owner_id)
                    .bind(&owner_token)
                    .fetch_one(&mut **tx)
                    .await?;
            Ok(id)
        })
    })
    .await?;
    assert!(!device_id.is_nil(), "the device was not registered");

    // The attacker registers the same token.
    let err = match with_app_participant(&pool, attacker_id, |tx| {
        Box::pin(async move {
            sqlx::query("SELECT register_push_device($1, 'android', $2, '1.0')")
                .bind(attacker_id)
                .bind(&stolen_token)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the attacker took the device token of another participant".into()),
        Err(err) => err,
    };

    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "unexpected error: {err}"
    );

    let owner: Uuid = sqlx::query_scalar("SELECT user_id FROM push_devices WHERE token = $1")
        .bind(&token)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        owner, owner_id,
        "the device token now belongs to the attacker"
    );

    Ok(())
}

/// A consumed magic link cannot be bound to another participant.
///
/// The link is the credential: whoever has it logs in. If the binding takes
/// the account from the caller, a leaked token id is an account takeover.
#[tokio::test]
async fn a_magic_link_cannot_be_bound_to_another_participant()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let owner_id = seed_user(&pool).await?;
    let attacker_id = seed_user(&pool).await?;

    let owner_email: String = sqlx::query_scalar("SELECT email::text FROM users WHERE id = $1")
        .bind(owner_id)
        .fetch_one(&pool)
        .await?;

    // The link of the owner, consumed.
    let token_id: Uuid = with_app_participant(&pool, owner_id, |tx| {
        Box::pin(async move {
            let id: Uuid =
                sqlx::query_scalar("SELECT insert_magic_link($1, $2, now() + interval '1 day')")
                    .bind(&owner_email)
                    .bind(Uuid::new_v4().as_bytes().to_vec())
                    .fetch_one(&mut **tx)
                    .await?;
            Ok(id)
        })
    })
    .await?;

    with_app_participant(&pool, owner_id, |tx| {
        Box::pin(async move {
            let _consumed: bool = sqlx::query_scalar("SELECT consume_magic_link($1)")
                .bind(token_id)
                .fetch_one(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    // The attacker binds the link of the owner to its own account.
    let err = match with_app_participant(&pool, attacker_id, |tx| {
        Box::pin(async move {
            let _bound: bool = sqlx::query_scalar("SELECT bind_magic_link_user($1, $2)")
                .bind(token_id)
                .bind(attacker_id)
                .fetch_one(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the attacker bound the link of another participant".into()),
        Err(err) => err,
    };

    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "unexpected error: {err}"
    );

    let bound: Option<Uuid> = sqlx::query_scalar("SELECT user_id FROM auth_tokens WHERE id = $1")
        .bind(token_id)
        .fetch_one(&pool)
        .await?;
    assert!(
        bound.is_none(),
        "the link of the owner is bound to another account"
    );

    // Y el correo del enlace es el de la cuenta que ata: ni la plataforma
    // puede ligar un enlace a una cuenta de otro correo.
    let other_token_id: Uuid = with_app_system(&pool, |tx| {
        Box::pin(async move {
            let id: Uuid =
                sqlx::query_scalar("SELECT insert_magic_link($1, $2, now() + interval '1 day')")
                    .bind("another@example.test")
                    .bind(Uuid::new_v4().as_bytes().to_vec())
                    .fetch_one(&mut **tx)
                    .await?;
            Ok(id)
        })
    })
    .await?;

    with_app_system(&pool, |tx| {
        Box::pin(async move {
            let _consumed: bool = sqlx::query_scalar("SELECT consume_magic_link($1)")
                .bind(other_token_id)
                .fetch_one(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    let err = match with_app_system(&pool, |tx| {
        Box::pin(async move {
            let _bound: bool = sqlx::query_scalar("SELECT bind_magic_link_user($1, $2)")
                .bind(other_token_id)
                .bind(owner_id)
                .fetch_one(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("a link of another email was bound to an account".into()),
        Err(err) => err,
    };

    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "unexpected error: {err}"
    );

    let bound: Option<Uuid> = sqlx::query_scalar("SELECT user_id FROM auth_tokens WHERE id = $1")
        .bind(other_token_id)
        .fetch_one(&pool)
        .await?;
    assert!(
        bound.is_none(),
        "the link is bound to an account of another email"
    );

    Ok(())
}

/// The tenant cannot write the counters of its own raffle.
///
/// The counters are denormalized: the database owns them and recomputes them
/// from the truth. A tenant that writes them makes the feed lie and the
/// reservation path report a number that nobody reserved.
#[tokio::test]
async fn the_tenant_cannot_write_the_counters_of_its_own_raffle()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;

    let err = match with_app_role(&pool, Some(business_id), None, |tx| {
        Box::pin(async move {
            sqlx::query(
                "UPDATE raffles SET numbers_reserved = 999, tickets_count = 999 WHERE id = $1",
            )
            .bind(raffle_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the tenant wrote the counters of its own raffle".into()),
        Err(err) => err,
    };

    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "unexpected error: {err}"
    );

    let (reserved, tickets_count): (i32, i32) =
        sqlx::query_as("SELECT numbers_reserved, tickets_count FROM raffles WHERE id = $1")
            .bind(raffle_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(reserved, 0, "the counter of numbers was written");
    assert_eq!(tickets_count, 0, "the counter of tickets was written");

    Ok(())
}

/// The catalogue of the write policies covers the exceptions.
///
/// The rule: a table that carries a `business_id` is written by the owner of
/// the row (or by the platform). The exceptions are the two writes the
/// product grants to a participant: the cancellation of its own ticket and
/// the read receipt of its own notification. Anything else in those two
/// policies is a hole, so their expressions are checked exactly.
///
/// The guard also covers `ticket_numbers`: it is a tenant table without a
/// `business_id` column, and the set of tables with the column did not reach
/// it.
#[tokio::test]
async fn the_catalogue_test_covers_the_exceptions() -> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;

    // The tenant tables: the ones that carry a `business_id`, the business
    // itself, and the participant tables that do not carry one.
    let tables: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT table_name FROM information_schema.columns
         WHERE table_schema = 'public' AND column_name = 'business_id'
        UNION
        SELECT 'businesses'
        UNION
        SELECT 'ticket_numbers'
        "#,
    )
    .fetch_all(&pool)
    .await?;

    assert!(
        tables.contains(&"ticket_numbers".to_string()),
        "the catalogue does not cover ticket_numbers"
    );

    // The exceptions, with the exact expression the product grants. The
    // strings are the rendering of `pg_get_expr`: a policy that changes is a
    // policy that the guard has to read again.
    let exceptions: Vec<(&str, &str, &str, &str)> = vec![
        (
            "tickets",
            "tickets_update",
            "((business_id = current_business_id()) OR (user_id = current_user_id()) OR is_system_context())",
            "((business_id = current_business_id()) OR is_system_context() OR (user_id IS NULL) OR ((user_id = current_user_id()) AND (status = 'cancelled'::text)))",
        ),
        (
            "notifications",
            "notifications_update",
            "((user_id = current_user_id()) OR is_system_context())",
            "((user_id = current_user_id()) OR is_system_context())",
        ),
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
    let mut changed: Vec<String> = Vec::new();

    for (table, policy, qual, with_check) in &policies {
        if !tables.contains(table) {
            continue;
        }

        match exceptions
            .iter()
            .find(|(t, p, _, _)| *t == table && *p == policy)
        {
            Some((_, _, expected_using, expected_with_check)) => {
                if qual.as_deref() != Some(*expected_using) {
                    changed.push(format!(
                        "{table}.{policy}: USING {:?}, expected {:?}",
                        qual, expected_using
                    ));
                }
                if with_check.as_deref() != Some(*expected_with_check) {
                    changed.push(format!(
                        "{table}.{policy}: WITH CHECK {:?}, expected {:?}",
                        with_check, expected_with_check
                    ));
                }
            }
            None => {
                for expression in [qual, with_check].into_iter().flatten() {
                    if expression.contains("current_user_id()") {
                        unbound.push(format!("{table}.{policy}: {expression}"));
                    }
                }
            }
        }
    }

    assert!(
        unbound.is_empty(),
        "the write policies of a tenant table have an unbound user_id branch: {}",
        unbound.join(" | ")
    );

    assert!(
        changed.is_empty(),
        "the exceptions of the catalogue have an expression the product does not grant: {}",
        changed.join(" | ")
    );

    // The exemptions cannot be used to make a policy disappear.
    for (table, policy, _, _) in &exceptions {
        assert!(
            policies
                .iter()
                .any(|(t, p, _, _)| t == table && p == policy),
            "the policy {table}.{policy} is gone"
        );
    }

    Ok(())
}

/// The platform can recompute the counters of a raffle.
///
/// The tenant cannot: that is the guarantee of `0040`. But the platform can,
/// and the trigger has to honour the system context the way its own comment
/// says. Without the branch, a platform job that recomputes the counters in
/// place would be refused with `42501`.
#[tokio::test]
async fn the_platform_can_recompute_the_counters_of_a_raffle()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let business_id = seed_business(&pool).await?;
    let raffle_id = seed_raffle(&pool, business_id, "published").await?;

    with_app_system(&pool, |tx| {
        Box::pin(async move {
            sqlx::query("UPDATE raffles SET numbers_reserved = 5, tickets_count = 3 WHERE id = $1")
                .bind(raffle_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    let (numbers, tickets): (i32, i32) =
        sqlx::query_as("SELECT numbers_reserved, tickets_count FROM raffles WHERE id = $1")
            .bind(raffle_id)
            .fetch_one(&pool)
            .await?;

    assert_eq!(numbers, 5, "the platform could not recompute the counters");
    assert_eq!(tickets, 3, "the platform could not recompute the counters");
    Ok(())
}

/// A participant cannot change the email or the password of its own account.
///
/// `status` and `deleted_at` have their own tests. These two are the same
/// class of refusal, and a regression on them would not be caught without a
/// test of their own.
#[tokio::test]
async fn a_participant_cannot_change_the_email_or_the_password_of_their_own_account()
-> Result<(), Box<dyn std::error::Error>> {
    let pool = test_pool().await?;
    ensure_app_role(&pool).await?;

    let user_id = seed_user(&pool).await?;

    // The email.
    let err = match with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query("UPDATE users SET email = 'back@example.test' WHERE id = $1")
                .bind(user_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the participant changed the email of their own account".into()),
        Err(err) => err,
    };
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "the refusal of the email did not carry the mapped SQLSTATE: {err}"
    );

    // The password.
    let err = match with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query("UPDATE users SET password_hash = 'forged-hash' WHERE id = $1")
                .bind(user_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await
    {
        Ok(_) => return Err("the participant changed the password of their own account".into()),
        Err(err) => err,
    };
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("42501"),
        "the refusal of the password did not carry the mapped SQLSTATE: {err}"
    );

    // The profile is still writable: the freeze is of the platform columns
    // only.
    with_app_participant(&pool, user_id, |tx| {
        Box::pin(async move {
            sqlx::query("UPDATE users SET display_name = 'New name' WHERE id = $1")
                .bind(user_id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    Ok(())
}
