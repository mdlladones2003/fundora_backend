use sqlx::PgPool;
use uuid::Uuid;
use crate::database::models::donation::{CreateDonation, Donation};

pub async fn create(pool: &PgPool, donor: Option<Uuid>, x: CreateDonation) -> Result<Donation, sqlx::Error> {
    sqlx::query_as(r#"
        INSERT INTO donations(campaign_id,donor_id,amount,currency,is_anonymous,message)
        SELECT $1,$2,$3,COALESCE($4,'PHP'),COALESCE($5,false),$6
        WHERE EXISTS(SELECT 1 FROM campaigns WHERE id=$1 AND status IN ('published','active'))
        RETURNING *
    "#)
    .bind(x.campaign_id)
    .bind(donor)
    .bind(x.amount)
    .bind(x.currency)
    .bind(x.is_anonymous)
    .bind(x.message)
    .fetch_one(pool).await
}

pub async fn list_for_campaign(pool: &PgPool, id: Uuid) -> Result<Vec<Donation>, sqlx::Error> {
    sqlx::query_as("
        SELECT * FROM donations WHERE campaign_id=$1 ORDER BY created_at DESC
    ")
    .bind(id)
    .fetch_all(pool).await
}
