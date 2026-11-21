use sqlx::PgPool;
use uuid::Uuid;
use crate::database::models::campaign::{Campaign, CreateCampaign, UpdateCampaign};

pub async fn list(pool: &PgPool) -> Result<Vec<Campaign>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM campaigns WHERE status IN ('published','active','completed') ORDER BY created_at DESC")
        .fetch_all(pool).await
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Campaign>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM campaigns WHERE id=$1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn create(pool: &PgPool, owner: Uuid, x: CreateCampaign) -> Result<Campaign, sqlx::Error> {
    sqlx::query_as(r#"
            INSERT INTO campaigns(organizer_id,title,description,image_url,goal_amount,currency,network,contract_address,starts_at,ends_at)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING *
        "#)
        .bind(owner)
        .bind(x.title)
        .bind(x.description)
        .bind(x.image_url)
        .bind(x.goal_amount)
        .bind(x.currency.unwrap_or_else(|| "PHP".into()))
        .bind(x.network)
        .bind(x.contract_address)
        .bind(x.starts_at)
        .bind(x.ends_at)
        .fetch_one(pool).await
}

pub async fn update(pool: &PgPool, id: Uuid, owner: Uuid, x: UpdateCampaign) -> Result<Option<Campaign>, sqlx::Error> {
    sqlx::query_as(r#"
            UPDATE campaigns SET title=COALESCE($3,title), description=COALESCE($4,description),
                image_url=COALESCE($5,image_url), goal_amount=COALESCE($6,goal_amount), status=COALESCE($7,status),
                starts_at=COALESCE($8,starts_at), ends_at=COALESCE($9,ends_at), updated_at=now()
            WHERE id=$1 AND organizer_id=$2 RETURNING *
        "#)
        .bind(id)
        .bind(owner)
        .bind(x.title)
        .bind(x.description)
        .bind(x.image_url)
        .bind(x.goal_amount)
        .bind(x.status)
        .bind(x.starts_at)
        .bind(x.ends_at)
        .fetch_optional(pool).await
}
