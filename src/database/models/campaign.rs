use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, FromRow)]
pub struct Campaign {
    pub id: Uuid,
    pub organizer_id: Uuid,
    pub title: String,
    pub description: String,
    pub image_url: Option<String>,
    pub goal_amount: Decimal,
    pub raised_amount: Decimal,
    pub currency: String,
    pub network: Option<String>,
    pub contract_address: Option<String>,
    pub status: String,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCampaign {
    pub title: String,
    pub description: String,
    pub image_url: Option<String>,
    pub goal_amount: Decimal,
    pub currency: Option<String>,
    pub network: Option<String>,
    pub contract_address: Option<String>,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCampaign {
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub goal_amount: Option<Decimal>,
    pub status: Option<String>,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
}
