use axum::{
    Extension, Json,
    extract::{Path, State},
};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::{AuthUser, has_role},
    database::{
        models::withdrawal::{CreateWithdrawal, ReviewWithdrawal, Withdrawal},
        repositories::activity_repository,
    },
    error::AppError,
};

#[derive(Debug, Serialize, FromRow)]
pub struct OrganizerWithdrawal {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub requester_id: Uuid,
    pub amount: Decimal,
    pub destination_address: String,
    pub purpose: String,
    pub status: String,
    pub review_reason: Option<String>,
    pub reviewed_by: Option<Uuid>,
    pub reviewed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub campaign_title: String,
    pub currency: String,
    pub available_balance: Decimal,
}

pub async fn list_mine(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<OrganizerWithdrawal>>, AppError> {
    if user.role != "organizer" {
        return Err(AppError::Forbidden);
    }
    let rows = sqlx::query_as::<_, OrganizerWithdrawal>(
        r#"
        SELECT w.*, c.title AS campaign_title, c.currency,
            GREATEST(c.raised_amount - COALESCE((
                SELECT SUM(reserved.amount)
                FROM withdrawals reserved
                WHERE reserved.campaign_id = c.id
                  AND reserved.status IN ('pending', 'approved')
            ), 0), 0) AS available_balance
        FROM withdrawals w
        JOIN campaigns c ON c.id = w.campaign_id
        WHERE w.requester_id = $1
        ORDER BY w.created_at DESC
        "#,
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

pub async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<CreateWithdrawal>,
) -> Result<Json<Withdrawal>, AppError> {
    if user.role != "organizer" {
        return Err(AppError::Forbidden);
    }
    if request.amount <= Decimal::ZERO
        || request.amount.fract() != Decimal::ZERO
        || request.destination_address.trim().is_empty()
        || request.destination_address.len() > 256
        || request.purpose.trim().is_empty()
        || request.purpose.len() > 1000
    {
        return Err(AppError::BadRequest(
            "A positive whole amount, destination, and purpose are required; destination and purpose have length limits".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    let campaign = sqlx::query_as::<_, (Decimal, String, String)>(
        "SELECT raised_amount, title, currency FROM campaigns WHERE id = $1 AND organizer_id = $2 FOR UPDATE",
    )
    .bind(request.campaign_id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let reserved = sqlx::query_scalar::<_, Decimal>(
        r#"
        SELECT COALESCE(SUM(amount), 0)
        FROM withdrawals
        WHERE campaign_id = $1 AND status IN ('pending', 'approved')
        "#,
    )
    .bind(request.campaign_id)
    .fetch_one(&mut *tx)
    .await?;
    let available = (campaign.0 - reserved).max(Decimal::ZERO);
    if request.amount > available {
        return Err(AppError::BadRequest(format!(
            "Requested amount exceeds the available balance of {} {}",
            available, campaign.2
        )));
    }

    let withdrawal = sqlx::query_as::<_, Withdrawal>(
        r#"
        INSERT INTO withdrawals(campaign_id, requester_id, amount, destination_address, purpose)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING *
        "#,
    )
    .bind(request.campaign_id)
    .bind(user.id)
    .bind(request.amount)
    .bind(request.destination_address)
    .bind(request.purpose)
    .fetch_one(&mut *tx)
    .await?;
    activity_repository::record_audit(
        &mut tx,
        Some(user.id),
        "withdrawal.requested",
        "withdrawal",
        Some(withdrawal.id),
        json!({
            "campaign_id": request.campaign_id,
            "amount": request.amount,
            "currency": campaign.2
        }),
    )
    .await?;
    activity_repository::notify_role(
        &mut tx,
        "administrator",
        "Withdrawal request needs review",
        &format!(
            "{} requested {} {} for {}",
            user.email, request.amount, campaign.2, campaign.1
        ),
        Some("/admin/withdrawals"),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(withdrawal))
}

pub async fn review(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(request): Json<ReviewWithdrawal>,
) -> Result<Json<Withdrawal>, AppError> {
    if !has_role(&user, &["administrator"]) {
        return Err(AppError::Forbidden);
    }
    let (status, action) = match request.decision.as_str() {
        "approve" => ("approved", "withdrawal.approved"),
        "reject" => ("rejected", "withdrawal.rejected"),
        _ => {
            return Err(AppError::BadRequest(
                "Decision must be approve or reject".into(),
            ));
        }
    };
    let reason = request
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    if status == "rejected" && reason.is_none() {
        return Err(AppError::BadRequest(
            "A rejection reason is required".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    let (campaign_id, requester_id, amount, current_status) =
        sqlx::query_as::<_, (Uuid, Uuid, Decimal, String)>(
            "SELECT campaign_id, requester_id, amount, status FROM withdrawals WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    if current_status != "pending" {
        return Err(AppError::BadRequest(
            "Only pending withdrawal requests can be reviewed".into(),
        ));
    }

    let raised_amount = sqlx::query_scalar::<_, Decimal>(
        "SELECT raised_amount FROM campaigns WHERE id = $1 FOR UPDATE",
    )
    .bind(campaign_id)
    .fetch_one(&mut *tx)
    .await?;
    if status == "approved" {
        let other_reserved = sqlx::query_scalar::<_, Decimal>(
            r#"
            SELECT COALESCE(SUM(amount), 0)
            FROM withdrawals
            WHERE campaign_id = $1 AND id <> $2 AND status IN ('pending', 'approved')
            "#,
        )
        .bind(campaign_id)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if amount > (raised_amount - other_reserved).max(Decimal::ZERO) {
            return Err(AppError::BadRequest(
                "The campaign no longer has enough available balance to approve this request"
                    .into(),
            ));
        }
    }

    let withdrawal = sqlx::query_as::<_, Withdrawal>(
        r#"
        UPDATE withdrawals
        SET status = $2, review_reason = $3, reviewed_by = $4, reviewed_at = NOW()
        WHERE id = $1 AND status = 'pending'
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(status)
    .bind(reason)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    activity_repository::record_audit(
        &mut tx,
        Some(user.id),
        action,
        "withdrawal",
        Some(id),
        json!({"decision": request.decision, "reason": reason, "amount": amount}),
    )
    .await?;
    activity_repository::notify_user(
        &mut tx,
        requester_id,
        if status == "approved" {
            "Withdrawal request approved"
        } else {
            "Withdrawal request rejected"
        },
        if status == "approved" {
            "The request is approved. No transfer has been initiated or completed."
        } else {
            reason.unwrap_or("The administrator rejected this request.")
        },
        Some("/organizer/withdrawals"),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(withdrawal))
}
