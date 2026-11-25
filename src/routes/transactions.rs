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
    extract::{Extension, State},
};

pub async fn record(
    State(s): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(x): Json<RecordTransaction>,
) -> Result<Json<BlockchainTransaction>, AppError> {
    if !has_role(&user, &["organizer", "administrator"]) {
        return Err(AppError::Forbidden);
    }

    if x.donation_id.is_some()
        || x.withdrawal_id.is_none()
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
            "Provide an approved withdrawal, a network, and a transaction hash. Donations must use the EVM verification endpoint.".into(),
        ));
    }

    let withdrawal_id = x
        .withdrawal_id
        .ok_or_else(|| AppError::BadRequest("An approved withdrawal is required.".into()))?;
    let owns_pending_record = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM withdrawals WHERE id = $1 AND requester_id = $2 AND status = 'approved')",
    )
    .bind(withdrawal_id)
    .bind(user.id)
    .fetch_one(&s.pool)
    .await?;
    if !owns_pending_record {
        return Err(AppError::Forbidden);
    }

    Ok(Json(repo::record(&s.pool, x).await?))
}
