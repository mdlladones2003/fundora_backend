use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub wallet_address: Option<String>,
    pub oauth_provider: Option<String>,
    pub oauth_subject: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

