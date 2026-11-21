use axum::{
    extract::{Path, State, Extension},
    http::HeaderMap, Json
};
use uuid::Uuid;
use crate::{
    app::AppState,
    auth::{AuthUser,has_role},
    database::{
        models::transaction::{RecordTransaction, BlockchainTransaction},
        repositories::transaction_repository as repo
    },
    error::AppError
};

pub async fn record(State(s): State<AppState>, Extension(user): Extension<AuthUser>, Json(x): Json<RecordTransaction>) -> Result<Json<BlockchainTransaction>, AppError> {
    if !has_role(&user, &["donor", "organizer", "administrator"]) {
        return Err(AppError::Forbidden);
    }
    
    if x.donation_id.is_none() && x.withdrawal_id.is_none() {
        return Err(AppError::BadRequest("donation_id or withdrawal_id is required".into()));
    }

    Ok(Json(repo::record(&s.pool,x).await?))
}

pub async fn confirm(State(s): State<AppState>, Path(id): Path<Uuid>, headers: HeaderMap, Json(body): Json<serde_json::Value>) -> Result<Json<BlockchainTransaction>, AppError> {
    let expected = &s.config.blockchain_webhook_secret;
    let supplied = headers.get("x-webhook-secret").and_then(|v|v.to_str().ok()).unwrap_or("");
    
    if expected.is_empty() || supplied != expected {
        return Err(AppError::Unauthorized);
    }
    
    let confirmations = body.get("confirmations").and_then(|v|v.as_i64()).unwrap_or(0) as i32;
    let block = body.get("block_number").and_then(|v|v.as_i64()).ok_or_else(||AppError::BadRequest("block_number required".into()))?;
    
    if confirmations < 1 {
        return Err(AppError::BadRequest("at least one confirmation is required".into()));
    }
    
    Ok(Json(repo::verify(&s.pool,id,confirmations,block).await?.ok_or(AppError::NotFound)?))
}
