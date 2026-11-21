use axum::{extract::State, Json};
use chrono::{Duration, Utc};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{Value};
use sqlx::Row;
use uuid::Uuid;
use crate::{
    app::AppState,
    auth::session,
    error::AppError
};

pub async fn complete_external_identity(state: &AppState, provider: &str, subject: &str, email: &str) -> Result<Json<Value>, AppError> {
    let email = email.trim().to_lowercase();
    let mut tx = state.pool.begin().await?;
    let existing = sqlx::query("SELECT user_id FROM auth_identities WHERE provider=$1 AND provider_subject=$2")
                        .bind(provider)
                        .bind(subject)
                        .fetch_optional(&mut *tx)
                        .await?;
    let uid: Uuid = if let Some(r) = existing {
        r.try_get("user_id")?
    } else {
        let found = sqlx::query("SELECT id FROM users WHERE lower(email)=$1")
                        .bind(&email)
                        .fetch_optional(&mut *tx)
                        .await?;
        
        let id: Uuid = if let Some(r)=found {
            r.try_get("id")?
        } else {
            sqlx::query_scalar("
                INSERT INTO users(email, display_name, is_active)
                VALUES($1,$2,TRUE) RETURNING id
            ")
            .bind(&email)
            .bind(email.split('@').next().unwrap_or("Fundora User"))
            .fetch_one(&mut *tx)
            .await?
        };
        sqlx::query("
            INSERT INTO auth_identities(user_id, provider, provider_subject, email, email_verified)
            VALUES($1,$2,$3,$4,TRUE) ON CONFLICT(provider,provider_subject) DO NOTHING
        ")
        .bind(id)
        .bind(provider)
        .bind(subject)
        .bind(&email)
        .execute(&mut *tx)
        .await?;
    
        id
    };

    tx.commit().await?; Ok(Json(session::create_session(state, uid).await?))
}

#[derive(Deserialize)]
pub struct NonceRequest {
    pub wallet_address: String
}

#[derive(Serialize)]
pub struct NonceResponse {
    pub nonce: String,
    pub expires_at: i64
}

pub async fn nonce(State(state): State<AppState>, Json(body): Json<NonceRequest>) -> Result<Json<NonceResponse>, AppError> {
    let address=body.wallet_address.trim().to_lowercase();

    if address.len()!=42 || !address.starts_with("0x") || !address[2..].chars().all(|c|c.is_ascii_hexdigit()) {
        return Err(AppError::BadRequest("Invalid Ethereum wallet address".into()));
    }

    let mut bytes=[0u8;16]; OsRng.fill_bytes(&mut bytes);
    let nonce=hex::encode(bytes);
    let exp=Utc::now() + Duration::minutes(5);

    sqlx::query("
        INSERT INTO siwe_nonces(wallet_address,nonce,expires_at)
        VALUES($1,$2,$3)
    ")
    .bind(address)
    .bind(&nonce)
    .bind(exp)
    .execute(&state.pool)
    .await?;
    
    Ok(Json(NonceResponse{nonce,expires_at:exp.timestamp()}))
}
#[derive(Deserialize)]
pub struct SiweVerifyRequest {
    pub message: String,
    pub signature: String
}

// Deliberately fail-closed: never authenticate from a client-posted wallet address without signature recovery.
pub async fn verify(State(_state): State<AppState>, Json(body): Json<SiweVerifyRequest>) -> Result<Json<Value>, AppError> {
    if body.message.trim().is_empty() || body.signature.trim().is_empty() {
        return Err(AppError::BadRequest("message and signature are required".into()));
    }

    Err(AppError::BadRequest("MetaMask verification is disabled until EIP-4361 parsing and EIP-191 signature recovery are installed".into()))
}
