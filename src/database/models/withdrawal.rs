use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, FromRow)]
pub struct Withdrawal {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub requester_id: Uuid,
    pub amount: Decimal,
    pub destination_address: String,
    pub purpose: String,
    pub status: String,
    pub review_reason: Option<String>,
    pub reviewed_by: Option<Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateWithdrawal {
    pub campaign_id: Uuid,
    pub amount: Decimal,
    pub destination_address: String,
    pub purpose: String,
}

#[derive(Debug, Deserialize)]
pub struct ReviewWithdrawal {
    pub decision: String,
    pub reason: Option<String>,
}
