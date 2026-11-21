use crate::{
    app::AppState,
    auth::{AuthUser, has_role},
    database::{
        models::transaction::{BlockchainTransaction, RecordTransaction},
        repositories::transaction_repository as repo,
    },
    error::AppError,
};
use axum::{
    Json,
    extract::{Extension, Path, State},
    http::HeaderMap,
};
use uuid::Uuid;

pub async fn record(
    State(s): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(x): Json<RecordTransaction>,
) -> Result<Json<BlockchainTransaction>, AppError> {
    if !has_role(&user, &["donor", "organizer"]) {
        return Err(AppError::Forbidden);
    }

    if x.donation_id.is_some() == x.withdrawal_id.is_some()
        || x.network.trim().is_empty()
        || x.network.len() > 80
        || x.tx_hash.trim().is_empty()
        || x.tx_hash.len() > 256
        || x.confirmations.is_some_and(|count| count < 0)
        || x.block_number.is_some_and(|number| number < 0)
        || x.from_address
            .as_deref()
            .is_some_and(|value| value.len() > 256)
        || x.to_address
            .as_deref()
            .is_some_and(|value| value.len() > 256)
    {
        return Err(AppError::BadRequest(
            "Provide exactly one pending donation or withdrawal, a network, and a transaction hash"
                .into(),
        ));
    }

    let owns_pending_record = if let Some(donation_id) = x.donation_id {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM donations WHERE id = $1 AND donor_id = $2 AND status = 'pending')",
        )
        .bind(donation_id)
        .bind(user.id)
        .fetch_one(&s.pool)
        .await?
    } else {
        let withdrawal_id = x
            .withdrawal_id
            .ok_or_else(|| AppError::BadRequest("A donation or withdrawal is required".into()))?;
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM withdrawals WHERE id = $1 AND requester_id = $2 AND status = 'approved')",
        )
        .bind(withdrawal_id)
        .bind(user.id)
        .fetch_one(&s.pool)
        .await?
    };
    if !owns_pending_record {
        return Err(AppError::Forbidden);
    }

    Ok(Json(repo::record(&s.pool, x).await?))
}

pub async fn confirm(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<BlockchainTransaction>, AppError> {
    let expected = &s.config.blockchain_webhook_secret;
    let supplied = headers
        .get("x-webhook-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if expected.is_empty() || supplied != expected {
        return Err(AppError::Unauthorized);
    }

    let confirmations = body
        .get("confirmations")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| AppError::BadRequest("confirmations required".into()))?;
    let confirmations = i32::try_from(confirmations).map_err(|_| {
        AppError::BadRequest("confirmations must be a positive 32-bit integer".into())
    })?;
    let block = body
        .get("block_number")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| AppError::BadRequest("block_number required".into()))?;

    if confirmations < 1 || block < 0 {
        return Err(AppError::BadRequest(
            "at least one confirmation and a non-negative block number are required".into(),
        ));
    }

    Ok(Json(
        repo::verify(&s.pool, id, confirmations, block)
            .await?
            .ok_or(AppError::NotFound)?,
    ))
}
