use axum::{
    Json,
    extract::{Extension, Path, State},
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, Row};
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::{AuthUser, has_role},
    database::{
        models::{
            campaign::{Campaign, UpdateCampaign},
            role::Role,
        },
        repositories::{activity_repository, campaign_repository},
    },
    error::AppError,
};

#[derive(Debug, Serialize, FromRow)]
pub struct AdminUser {
    pub id: Uuid,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub wallet_address: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub role: String,
}

#[derive(Debug, Serialize, FromRow)]
pub struct AdminDonation {
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
    pub campaign_title: String,
    pub donor_email: Option<String>,
    pub transaction_hash: Option<String>,
    pub transaction_network: Option<String>,
    pub transaction_status: Option<String>,
    pub transaction_verified: Option<bool>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct AdminWithdrawal {
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
    pub campaign_title: String,
    pub currency: String,
    pub available_balance: Decimal,
    pub requester_email: String,
}

#[derive(Debug, Serialize, FromRow)]
pub struct AdminAuditLog {
    pub id: Uuid,
    pub actor_id: Option<Uuid>,
    pub actor_email: Option<String>,
    pub action: String,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    pub details: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct AdminTransaction {
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
    pub amount: Option<Decimal>,
    pub currency: Option<String>,
    pub campaign_title: Option<String>,
    pub review_note: Option<String>,
    pub flagged_by: Option<Uuid>,
    pub flagged_at: Option<DateTime<Utc>>,
}

fn require_admin(user: &AuthUser) -> Result<(), AppError> {
    if has_role(user, &["administrator"]) {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

pub async fn list_campaigns(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<Campaign>>, AppError> {
    require_admin(&user)?;
    Ok(Json(campaign_repository::list_all(&state.pool).await?))
}

pub async fn update_campaign(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateCampaign>,
) -> Result<Json<Campaign>, AppError> {
    require_admin(&user)?;
    if body
        .title
        .as_ref()
        .is_some_and(|title| title.trim().is_empty())
        || body
            .description
            .as_ref()
            .is_some_and(|description| description.trim().is_empty())
        || body
            .goal_amount
            .is_some_and(|amount| amount <= Decimal::ZERO || amount.fract() != Decimal::ZERO)
        || body
            .category
            .as_deref()
            .is_some_and(|value| value.len() > 80)
        || body
            .beneficiary_name
            .as_deref()
            .is_some_and(|value| value.len() > 160)
        || body
            .beneficiary_details
            .as_deref()
            .is_some_and(|value| value.len() > 4000)
        || body
            .supporting_info
            .as_deref()
            .is_some_and(|value| value.len() > 4000)
        || body
            .image_url
            .as_deref()
            .is_some_and(|value| value.len() > 2048)
    {
        return Err(AppError::BadRequest(
            "Campaign title and description are required, and goal_amount must be positive".into(),
        ));
    }
    if body.status.is_some() {
        return Err(AppError::BadRequest(
            "Campaign status must be changed through the campaign review action".into(),
        ));
    }

    let audit_details = json!({
        "title": &body.title,
        "description": &body.description,
        "category": &body.category,
        "beneficiary_name": &body.beneficiary_name,
        "beneficiary_details": &body.beneficiary_details,
        "supporting_info": &body.supporting_info,
        "goal_amount": body.goal_amount
    });
    let mut tx = state.pool.begin().await?;
    let campaign = campaign_repository::update_admin(&mut tx, id, body)
        .await?
        .ok_or(AppError::NotFound)?;
    activity_repository::record_audit(
        &mut tx,
        Some(user.id),
        "campaign.updated",
        "campaign",
        Some(id),
        audit_details,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(campaign))
}

#[derive(Debug, Deserialize)]
pub struct ReviewCampaign {
    pub decision: String,
    pub reason: Option<String>,
}

pub async fn review_campaign(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(body): Json<ReviewCampaign>,
) -> Result<Json<Campaign>, AppError> {
    require_admin(&user)?;
    let (status, action, title) = match body.decision.as_str() {
        "approve" => ("published", "campaign.approved", "Campaign approved"),
        "reject" => ("rejected", "campaign.rejected", "Campaign not approved"),
        "request_changes" => (
            "changes_requested",
            "campaign.changes_requested",
            "Campaign changes requested",
        ),
        _ => {
            return Err(AppError::BadRequest(
                "Decision must be approve, reject, or request_changes".into(),
            ));
        }
    };
    let reason = body
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    if status != "published" && reason.is_none() {
        return Err(AppError::BadRequest(
            "A reason is required when rejecting or returning a campaign".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    let campaign = sqlx::query_as::<_, Campaign>(
        r#"
        UPDATE campaigns
        SET status = $2, review_feedback = $3, reviewed_by = $4, reviewed_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND status = 'pending_review'
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(status)
    .bind(reason)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    activity_repository::record_audit(
        &mut tx,
        Some(user.id),
        action,
        "campaign",
        Some(id),
        json!({"decision": body.decision, "reason": reason}),
    )
    .await?;
    activity_repository::notify_user(
        &mut tx,
        campaign.organizer_id,
        title,
        reason.unwrap_or("Your campaign is now visible to donors."),
        Some("/dashboard/organizer"),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(campaign))
}

pub async fn list_audit_logs(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<AdminAuditLog>>, AppError> {
    require_admin(&user)?;
    let logs = sqlx::query_as::<_, AdminAuditLog>(
        r#"
        SELECT l.id, l.actor_id, u.email AS actor_email, l.action, l.entity_type,
               l.entity_id, l.details, l.created_at
        FROM audit_logs l
        LEFT JOIN users u ON u.id = l.actor_id
        ORDER BY l.created_at DESC
        LIMIT 500
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(logs))
}

pub async fn list_transactions(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<AdminTransaction>>, AppError> {
    require_admin(&user)?;
    let transactions = sqlx::query_as::<_, AdminTransaction>(
        r#"
        SELECT t.id, t.donation_id, t.withdrawal_id, t.network, t.tx_hash,
               t.from_address, t.to_address, t.block_number, t.confirmations,
               t.status, t.verified, t.created_at, t.confirmed_at,
               COALESCE(d.amount, w.amount) AS amount,
               COALESCE(d.currency, wc.currency, dc.currency) AS currency,
               COALESCE(dc.title, wc.title) AS campaign_title,
               t.review_note, t.flagged_by, t.flagged_at
        FROM blockchain_transactions t
        LEFT JOIN donations d ON d.id = t.donation_id
        LEFT JOIN campaigns dc ON dc.id = d.campaign_id
        LEFT JOIN withdrawals w ON w.id = t.withdrawal_id
        LEFT JOIN campaigns wc ON wc.id = w.campaign_id
        ORDER BY t.created_at DESC
        LIMIT 1000
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(transactions))
}

#[derive(Debug, Deserialize)]
pub struct FlagTransaction {
    pub note: Option<String>,
}

pub async fn flag_transaction(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(body): Json<FlagTransaction>,
) -> Result<Json<AdminTransaction>, AppError> {
    require_admin(&user)?;
    let note = body
        .note
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    if note.is_some_and(|text| text.len() > 2000) {
        return Err(AppError::BadRequest(
            "The review note must be 2000 characters or fewer".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    let updated = sqlx::query(
        r#"
        UPDATE blockchain_transactions
        SET review_note = $2,
            flagged_by = CASE WHEN $2 IS NULL THEN NULL ELSE $3 END,
            flagged_at = CASE WHEN $2 IS NULL THEN NULL ELSE NOW() END
        WHERE id = $1
        RETURNING donation_id, withdrawal_id
        "#,
    )
    .bind(id)
    .bind(note)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    activity_repository::record_audit(
        &mut tx,
        Some(user.id),
        if note.is_some() {
            "transaction.flagged"
        } else {
            "transaction.flag_cleared"
        },
        "blockchain_transaction",
        Some(id),
        json!({"note": note, "donation_id": updated.try_get::<Option<Uuid>, _>("donation_id")?, "withdrawal_id": updated.try_get::<Option<Uuid>, _>("withdrawal_id")?}),
    )
    .await?;
    tx.commit().await?;
    get_admin_transaction(&state, id).await.map(Json)
}

async fn get_admin_transaction(state: &AppState, id: Uuid) -> Result<AdminTransaction, AppError> {
    sqlx::query_as::<_, AdminTransaction>(
        r#"
        SELECT t.id, t.donation_id, t.withdrawal_id, t.network, t.tx_hash,
               t.from_address, t.to_address, t.block_number, t.confirmations,
               t.status, t.verified, t.created_at, t.confirmed_at,
               COALESCE(d.amount, w.amount) AS amount,
               COALESCE(d.currency, wc.currency, dc.currency) AS currency,
               COALESCE(dc.title, wc.title) AS campaign_title,
               t.review_note, t.flagged_by, t.flagged_at
        FROM blockchain_transactions t
        LEFT JOIN donations d ON d.id = t.donation_id
        LEFT JOIN campaigns dc ON dc.id = d.campaign_id
        LEFT JOIN withdrawals w ON w.id = t.withdrawal_id
        LEFT JOIN campaigns wc ON wc.id = w.campaign_id
        WHERE t.id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)
}

pub async fn list_users(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<AdminUser>>, AppError> {
    require_admin(&user)?;
    let users = sqlx::query_as::<_, AdminUser>(r#"
        SELECT u.id,u.email,u.display_name,u.avatar_url,u.wallet_address,u.is_active,u.created_at,
            COALESCE((
                SELECT r.name FROM user_roles ur
                JOIN roles r ON r.id=ur.role_id
                WHERE ur.user_id=u.id
                ORDER BY CASE r.name WHEN 'administrator' THEN 3 WHEN 'organizer' THEN 2 WHEN 'donor' THEN 1 ELSE 0 END DESC
                LIMIT 1
            ),'donor') AS role
        FROM users u
        ORDER BY u.created_at DESC
    "#)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(users))
}

#[derive(Debug, Deserialize)]
pub struct UpdateUser {
    pub role: Option<String>,
    pub is_active: Option<bool>,
}

pub async fn update_user(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateUser>,
) -> Result<Json<AdminUser>, AppError> {
    require_admin(&user)?;
    let requested_role = body.role.as_deref();
    if let Some(role) = requested_role {
        if !matches!(
            Role::from_name(role),
            Some(Role::Donor | Role::Organizer | Role::Administrator)
        ) {
            return Err(AppError::BadRequest(
                "Role must be donor, organizer, or administrator".into(),
            ));
        }
    }
    if body.role.is_none() && body.is_active.is_none() {
        return Err(AppError::BadRequest(
            "Provide a role or account status to update".into(),
        ));
    }
    if id == user.id
        && (body.is_active == Some(false)
            || requested_role.is_some_and(|role| role != "administrator"))
    {
        return Err(AppError::BadRequest(
            "You cannot remove your own administrator access".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    sqlx::query("LOCK TABLE user_roles IN EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    let target_active =
        sqlx::query_scalar::<_, bool>("SELECT is_active FROM users WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AppError::NotFound)?;
    let current_role = sqlx::query_scalar::<_, String>(r#"
        SELECT r.name FROM user_roles ur
        JOIN roles r ON r.id=ur.role_id
        WHERE ur.user_id=$1
        ORDER BY CASE r.name WHEN 'administrator' THEN 3 WHEN 'organizer' THEN 2 WHEN 'donor' THEN 1 ELSE 0 END DESC
        LIMIT 1
    "#)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or_else(|| "donor".into());
    if requested_role == Some("organizer") && current_role != "organizer" {
        return Err(AppError::BadRequest(
            "Approve a pending organizer access request to grant this role".into(),
        ));
    }
    let next_role = requested_role.unwrap_or(&current_role);
    let next_active = body.is_active.unwrap_or(target_active);
    if target_active
        && current_role == "administrator"
        && (!next_active || next_role != "administrator")
    {
        let active_admins = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(DISTINCT u.id) FROM users u
            JOIN user_roles ur ON ur.user_id=u.id
            JOIN roles r ON r.id=ur.role_id
            WHERE u.is_active=TRUE AND r.name='administrator'
        "#,
        )
        .fetch_one(&mut *tx)
        .await?;
        if active_admins <= 1 {
            return Err(AppError::BadRequest(
                "The last active administrator cannot be removed".into(),
            ));
        }
    }

    if let Some(active) = body.is_active {
        sqlx::query("UPDATE users SET is_active=$2,updated_at=NOW() WHERE id=$1")
            .bind(id)
            .bind(active)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(role) = requested_role {
        sqlx::query("DELETE FROM user_roles WHERE user_id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO user_roles(user_id,role_id) SELECT $1,id FROM roles WHERE name=$2",
        )
        .bind(id)
        .bind(role)
        .execute(&mut *tx)
        .await?;
    }
    activity_repository::record_audit(
        &mut tx,
        Some(user.id),
        "user.updated",
        "user",
        Some(id),
        json!({
            "role": requested_role.unwrap_or(&current_role),
            "is_active": body.is_active.unwrap_or(target_active)
        }),
    )
    .await?;
    activity_repository::notify_user(
        &mut tx,
        id,
        "Account access updated",
        &format!(
            "An administrator updated your account to role '{}' and {}.",
            next_role,
            if next_active { "active" } else { "inactive" }
        ),
        Some("/profile"),
    )
    .await?;
    tx.commit().await?;

    get_user(&state, id).await.map(Json)
}

async fn get_user(state: &AppState, id: Uuid) -> Result<AdminUser, AppError> {
    sqlx::query_as::<_, AdminUser>(r#"
        SELECT u.id,u.email,u.display_name,u.avatar_url,u.wallet_address,u.is_active,u.created_at,
            COALESCE((
                SELECT r.name FROM user_roles ur
                JOIN roles r ON r.id=ur.role_id
                WHERE ur.user_id=u.id
                ORDER BY CASE r.name WHEN 'administrator' THEN 3 WHEN 'organizer' THEN 2 WHEN 'donor' THEN 1 ELSE 0 END DESC
                LIMIT 1
            ),'donor') AS role
        FROM users u WHERE u.id=$1
    "#)
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)
}

pub async fn list_donations(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<AdminDonation>>, AppError> {
    require_admin(&user)?;
    let donations = sqlx::query_as::<_, AdminDonation>(
        r#"
        SELECT d.*,c.title AS campaign_title,u.email AS donor_email,
               t.tx_hash AS transaction_hash,t.network AS transaction_network,
               t.status AS transaction_status,t.verified AS transaction_verified
        FROM donations d
        JOIN campaigns c ON c.id=d.campaign_id
        LEFT JOIN users u ON u.id=d.donor_id
        LEFT JOIN LATERAL (
            SELECT tx_hash FROM blockchain_transactions
            WHERE donation_id=d.id ORDER BY created_at DESC LIMIT 1
        ) t ON TRUE
        ORDER BY d.created_at DESC
    "#,
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(donations))
}

pub async fn list_withdrawals(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<AdminWithdrawal>>, AppError> {
    require_admin(&user)?;
    let withdrawals = sqlx::query_as::<_, AdminWithdrawal>(
        r#"
        SELECT w.*, c.title AS campaign_title, c.currency, u.email AS requester_email,
            GREATEST(c.raised_amount - COALESCE((
                SELECT SUM(reserved.amount)
                FROM withdrawals reserved
                WHERE reserved.campaign_id = c.id
                  AND reserved.status IN ('pending', 'approved')
            ), 0), 0) AS available_balance
        FROM withdrawals w
        JOIN campaigns c ON c.id=w.campaign_id
        JOIN users u ON u.id=w.requester_id
        ORDER BY w.created_at DESC
    "#,
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(withdrawals))
}
