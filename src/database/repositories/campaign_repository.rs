use crate::database::models::campaign::{Campaign, CreateCampaign, UpdateCampaign};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub async fn list(pool: &PgPool) -> Result<Vec<Campaign>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT c.id, c.organizer_id, c.title, c.description, c.category,
               c.beneficiary_name, c.beneficiary_details, c.supporting_info,
               c.image_url, c.goal_amount, c.raised_amount, c.currency,
               c.network, c.contract_address, c.status,
               NULL::text AS review_feedback, NULL::uuid AS reviewed_by,
               NULL::timestamptz AS reviewed_at, c.starts_at, c.ends_at,
               c.created_at, c.updated_at,
               (SELECT COUNT(DISTINCT d.donor_id)::bigint
                FROM donations d
                WHERE d.campaign_id = c.id AND d.status = 'confirmed') AS donor_count
        FROM campaigns c
        WHERE c.status IN ('published','active','completed')
        ORDER BY c.created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await
}

pub async fn list_for_organizer(
    pool: &PgPool,
    organizer_id: Uuid,
) -> Result<Vec<Campaign>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT c.*,
               (SELECT COUNT(DISTINCT d.donor_id)::bigint
                FROM donations d
                WHERE d.campaign_id = c.id AND d.status = 'confirmed') AS donor_count
        FROM campaigns c
        WHERE c.organizer_id=$1
        ORDER BY c.created_at DESC
        "#,
    )
    .bind(organizer_id)
    .fetch_all(pool)
    .await
}

pub async fn create(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    x: CreateCampaign,
) -> Result<Campaign, sqlx::Error> {
    sqlx::query_as(
        r#"
            INSERT INTO campaigns(
                organizer_id,title,description,category,beneficiary_name,beneficiary_details,
                supporting_info,image_url,goal_amount,currency,network,contract_address,
                starts_at,ends_at,status
            )
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,
                CASE WHEN $15 THEN 'pending_review' ELSE 'draft' END
            ) RETURNING *
        "#,
    )
    .bind(owner)
    .bind(x.title)
    .bind(x.description)
    .bind(x.category)
    .bind(x.beneficiary_name)
    .bind(x.beneficiary_details)
    .bind(x.supporting_info)
    .bind(x.image_url)
    .bind(x.goal_amount)
    .bind(x.currency.unwrap_or_else(|| "PHP".into()))
    .bind(x.network)
    .bind(x.contract_address)
    .bind(x.starts_at)
    .bind(x.ends_at)
    .bind(x.submit_for_review.unwrap_or(false))
    .fetch_one(&mut **tx)
    .await
}

pub async fn update(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    owner: Uuid,
    x: UpdateCampaign,
) -> Result<Option<Campaign>, sqlx::Error> {
    sqlx::query_as(r#"
            UPDATE campaigns SET title=COALESCE($3,title), description=COALESCE($4,description),
                category=CASE WHEN $5 IS NULL THEN category ELSE NULLIF(BTRIM($5), '') END,
                beneficiary_name=CASE WHEN $6 IS NULL THEN beneficiary_name ELSE NULLIF(BTRIM($6), '') END,
                beneficiary_details=CASE WHEN $7 IS NULL THEN beneficiary_details ELSE NULLIF(BTRIM($7), '') END,
                supporting_info=CASE WHEN $8 IS NULL THEN supporting_info ELSE NULLIF(BTRIM($8), '') END,
                image_url=CASE WHEN $9 IS NULL THEN image_url ELSE NULLIF(BTRIM($9), '') END,
                goal_amount=COALESCE($10,goal_amount),
                status=COALESCE($11,status), starts_at=COALESCE($12,starts_at),
                ends_at=COALESCE($13,ends_at),
                review_feedback=CASE WHEN $11='pending_review' THEN NULL ELSE review_feedback END,
                reviewed_by=CASE WHEN $11='pending_review' THEN NULL ELSE reviewed_by END,
                reviewed_at=CASE WHEN $11='pending_review' THEN NULL ELSE reviewed_at END,
                updated_at=now()
            WHERE id=$1 AND organizer_id=$2 RETURNING *
        "#)
        .bind(id)
        .bind(owner)
        .bind(x.title)
        .bind(x.description)
        .bind(x.category)
        .bind(x.beneficiary_name)
        .bind(x.beneficiary_details)
        .bind(x.supporting_info)
        .bind(x.image_url)
        .bind(x.goal_amount)
        .bind(x.status)
        .bind(x.starts_at)
        .bind(x.ends_at)
        .fetch_optional(&mut **tx).await
}

pub async fn update_admin(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    x: UpdateCampaign,
) -> Result<Option<Campaign>, sqlx::Error> {
    sqlx::query_as(r#"
            UPDATE campaigns SET title=COALESCE($2,title), description=COALESCE($3,description),
                category=CASE WHEN $4 IS NULL THEN category ELSE NULLIF(BTRIM($4), '') END,
                beneficiary_name=CASE WHEN $5 IS NULL THEN beneficiary_name ELSE NULLIF(BTRIM($5), '') END,
                beneficiary_details=CASE WHEN $6 IS NULL THEN beneficiary_details ELSE NULLIF(BTRIM($6), '') END,
                supporting_info=CASE WHEN $7 IS NULL THEN supporting_info ELSE NULLIF(BTRIM($7), '') END,
                image_url=CASE WHEN $8 IS NULL THEN image_url ELSE NULLIF(BTRIM($8), '') END,
                goal_amount=COALESCE($9,goal_amount),
                status=COALESCE($10,status), starts_at=COALESCE($11,starts_at),
                ends_at=COALESCE($12,ends_at), updated_at=now()
            WHERE id=$1 RETURNING *
        "#)
        .bind(id)
        .bind(x.title)
        .bind(x.description)
        .bind(x.category)
        .bind(x.beneficiary_name)
        .bind(x.beneficiary_details)
        .bind(x.supporting_info)
        .bind(x.image_url)
        .bind(x.goal_amount)
        .bind(x.status)
        .bind(x.starts_at)
        .bind(x.ends_at)
        .fetch_optional(&mut **tx)
        .await
}
