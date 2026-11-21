use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, FromRow)]
pub struct Donation {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub donor_id: Option<Uuid>,
    pub amount: Decimal,
    pub currency: String,
    pub status: String,
    pub is_anonymous: bool,
    pub message: Option<String>, 
    pub created_at: DateTime<Utc>,
    pub confirmed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDonation {
    pub campaign_id: Uuid,
    pub amount: Decimal,
    pub currency: Option<String>,
    pub is_anonymous: Option<bool>,
    pub message: Option<String>,
}
