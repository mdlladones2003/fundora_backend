use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, FromRow)]
pub struct BlockchainTransaction {
    pub id: Uuid,
    pub donation_id: Option<Uuid>,
    pub withdrawal_id: Option<Uuid>,
    pub network: String,
    pub tx_hash: String,
    pub from_address: Option<String>,
    pub to_address: Option<String>,
    pub block_number: Option<i64>,
    pub confirmations: i32,
    pub status: String,
    pub verified: bool,
    pub created_at: DateTime<Utc>,
    pub confirmed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct RecordTransaction {
    pub donation_id: Option<Uuid>,
    pub withdrawal_id: Option<Uuid>,
    pub network: String,
    pub tx_hash: String,
    pub from_address: Option<String>,
    pub to_address: Option<String>,
    pub block_number: Option<i64>,
    pub confirmations: Option<i32>,
}
