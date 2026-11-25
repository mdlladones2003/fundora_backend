use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, FromRow)]
pub struct DonationHistory {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub donor_id: Option<Uuid>,
    pub donor_wallet_address: Option<String>,
    pub recipient_wallet_address: Option<String>,
    pub amount: Decimal,
    pub currency: String,
    pub payment_amount: Option<Decimal>,
    pub payment_currency: Option<String>,
    pub exchange_rate: Option<Decimal>,
    pub exchange_rate_provider: Option<String>,
    pub exchange_rate_at: Option<DateTime<Utc>>,
    pub status: String,
    pub payment_method: Option<String>,
    pub payment_provider: Option<String>,
    pub payment_reference: Option<String>,
    pub is_anonymous: bool,
    pub message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub campaign_title: String,
    pub transaction_hash: Option<String>,
    pub transaction_network: Option<String>,
    pub transaction_status: Option<String>,
    pub transaction_verified: Option<bool>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct DonationReceipt {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub donor_id: Option<Uuid>,
    pub donor_wallet_address: Option<String>,
    pub recipient_wallet_address: Option<String>,
    pub amount: Decimal,
    pub currency: String,
    pub payment_amount: Option<Decimal>,
    pub payment_currency: Option<String>,
    pub exchange_rate: Option<Decimal>,
    pub exchange_rate_provider: Option<String>,
    pub exchange_rate_at: Option<DateTime<Utc>>,
    pub status: String,
    pub payment_method: Option<String>,
    pub payment_provider: Option<String>,
    pub payment_reference: Option<String>,
    pub is_anonymous: bool,
    pub message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub campaign_title: String,
    pub transaction_hash: Option<String>,
    pub transaction_network: Option<String>,
    pub transaction_status: Option<String>,
    pub transaction_verified: Option<bool>,
    pub block_number: Option<i64>,
    pub confirmations: Option<i32>,
    pub transaction_confirmed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct DonationVerification {
    pub donation_id: Uuid,
    pub status: String,
    pub transaction_hash: String,
    pub confirmations: i32,
}
