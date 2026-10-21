//! Consented contacts of a business (`docs/data-model.md` §6.4).
//!
//! The query is the monetisation path: a business asks for the participants
//! that allowed being contacted, and the answer has to honour a revocation
//! that happened after the participant reserved a ticket.

use chrono::{DateTime, Utc};
use sqlx::postgres::PgPool;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use super::RepositoryError;
use crate::pool::TenantContext;

/// A participant that granted `share_marketing` and holds a live ticket.
#[derive(Debug, Clone, FromRow)]
pub struct ConsentedContact {
    pub user_id: Uuid,
    pub display_name: String,
    pub email: String,
    pub phone_e164: Option<String>,
    pub raffle_id: Uuid,
    pub ticket_code: String,
    pub participated_at: DateTime<Utc>,
    pub consented_at: DateTime<Utc>,
}

/// The participants of a business that granted `share_marketing`.
///
/// The `DISTINCT` and the "last consent wins" subquery resolve the revocation:
/// the newest consent of the participant is the one that counts, so a
/// revocation that happened after the ticket removes the contact from the
/// export.
pub async fn consented_contacts(
    tx: &mut Transaction<'_, Postgres>,
    business_id: Uuid,
) -> Result<Vec<ConsentedContact>, RepositoryError> {
    Ok(sqlx::query_as!(
        ConsentedContact,
        r#"
        SELECT DISTINCT
            u.id AS user_id,
            u.display_name,
            u.email AS email,
            u.phone_e164,
            t.raffle_id,
            t.code AS ticket_code,
            t.created_at AS participated_at,
            c.created_at AS consented_at
        FROM tickets t
        JOIN users u ON u.id = t.user_id
        JOIN consent_records c
             ON c.user_id = u.id
            AND c.business_id = t.business_id
            AND c.scope = 'share_marketing'
            AND c.granted = true
        WHERE t.business_id = $1
          AND t.status <> 'cancelled'
          AND t.created_at > (
              SELECT max(c2.created_at) FROM consent_records c2
              WHERE c2.user_id = u.id AND c2.business_id = t.business_id
                AND c2.scope = 'share_marketing' AND c2.granted = true
          )
          AND (
              SELECT c3.granted
              FROM consent_records c3
              WHERE c3.user_id = u.id AND c3.business_id = t.business_id
                AND c3.scope = 'share_marketing'
              ORDER BY c3.created_at DESC
              LIMIT 1
          ) = true
        ORDER BY t.created_at DESC
        "#,
        business_id
    )
    .fetch_all(&mut **tx)
    .await?)
}

/// Pool-level wrapper for [`consented_contacts`].
pub async fn consented_contacts_in_tenant(
    pool: &PgPool,
    ctx: TenantContext,
    business_id: Uuid,
) -> Result<Vec<ConsentedContact>, RepositoryError> {
    crate::pool::with_tenant(pool, ctx, move |tx| {
        Box::pin(async move { consented_contacts(tx, business_id).await })
    })
    .await
}
