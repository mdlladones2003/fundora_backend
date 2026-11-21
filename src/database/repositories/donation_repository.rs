use crate::database::models::donation::{CreateDonation, Donation};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn create(
    pool: &PgPool,
    donor: Option<Uuid>,
    x: CreateDonation,
) -> Result<Donation, sqlx::Error> {
    sqlx::query_as(
        r#"
        INSERT INTO donations(campaign_id,donor_id,amount,currency,is_anonymous,message)
        SELECT c.id,$2,$3,c.currency,COALESCE($4,false),$5
        FROM campaigns c
        WHERE c.id=$1 AND c.status IN ('published','active')
        RETURNING *
    "#,
    )
    .bind(x.campaign_id)
    .bind(donor)
    .bind(x.amount)
    .bind(x.is_anonymous)
    .bind(x.message)
    .fetch_one(pool)
    .await
}
