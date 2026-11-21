use axum::{
    Extension, Json,
    extract::{Path, State},
};
use chrono::Utc;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::{AuthUser, has_role},
    database::{
        models::campaign::{Campaign, CampaignDetails, CreateCampaign, UpdateCampaign},
        repositories::{activity_repository, campaign_repository as repo},
    },
    error::AppError,
};
use serde_json::json;

pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<Campaign>>, AppError> {
    Ok(Json(repo::list(&state.pool).await?))
}

pub async fn list_mine(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<Campaign>>, AppError> {
    if !has_role(&user, &["organizer", "administrator"]) {
        return Err(AppError::Forbidden);
    }
    Ok(Json(repo::list_for_organizer(&state.pool, user.id).await?))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<CampaignDetails>, AppError> {
    let campaign = sqlx::query_as::<_, CampaignDetails>(
        r#"
        SELECT c.id, c.organizer_id, u.display_name AS organizer_name,
               c.title, c.description, c.category, c.beneficiary_name,
               c.beneficiary_details, c.supporting_info, c.image_url,
               c.goal_amount, c.raised_amount, c.currency, c.network,
               c.contract_address, c.status, NULL::text AS review_feedback,
               NULL::uuid AS reviewed_by, NULL::timestamptz AS reviewed_at,
               c.starts_at, c.ends_at, c.created_at, c.updated_at
        FROM campaigns c
        JOIN users u ON u.id = c.organizer_id
        WHERE c.id = $1 AND c.status IN ('published', 'active', 'completed')
        "#,
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(campaign))
}

fn validate_schedule(
    starts_at: Option<&chrono::DateTime<Utc>>,
    ends_at: Option<&chrono::DateTime<Utc>>,
    require_future: bool,
) -> Result<(), AppError> {
    if (require_future
        && (starts_at.is_some_and(|start| start <= &Utc::now())
            || ends_at.is_some_and(|end| end <= &Utc::now())))
        || matches!((starts_at, ends_at), (Some(start), Some(end)) if end <= start)
    {
        return Err(AppError::BadRequest(
            "Campaign dates must be in the future, and the end date must follow the start date"
                .into(),
        ));
    }
    Ok(())
}

pub async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(campaign): Json<CreateCampaign>,
) -> Result<Json<Campaign>, AppError> {
    if !has_role(&user, &["organizer", "administrator"]) {
        return Err(AppError::Forbidden);
    }
    validate_schedule(campaign.starts_at.as_ref(), campaign.ends_at.as_ref(), true)?;
    if campaign.title.trim().is_empty()
        || campaign.title.len() > 160
        || campaign.description.trim().is_empty()
        || campaign.description.len() > 10_000
        || campaign.goal_amount <= Decimal::ZERO
        || campaign.goal_amount.fract() != Decimal::ZERO
        || campaign
            .category
            .as_deref()
            .is_some_and(|value| value.len() > 80)
        || campaign
            .beneficiary_name
            .as_deref()
            .is_some_and(|value| value.len() > 160)
        || campaign
            .beneficiary_details
            .as_deref()
            .is_some_and(|value| value.len() > 4000)
        || campaign
            .supporting_info
            .as_deref()
            .is_some_and(|value| value.len() > 4000)
        || campaign
            .image_url
            .as_deref()
            .is_some_and(|value| value.len() > 2048)
        || campaign
            .currency
            .as_deref()
            .is_some_and(|value| !matches!(value, "PHP" | "USD" | "EUR"))
    {
        return Err(AppError::BadRequest(
            "A title, description, and positive goal amount are required".into(),
        ));
    }
    let submit_for_review = campaign.submit_for_review.unwrap_or(false);
    if submit_for_review
        && (campaign.category.as_deref().is_none_or(str::is_empty)
            || campaign
                .beneficiary_name
                .as_deref()
                .is_none_or(str::is_empty)
            || campaign.ends_at.is_none())
    {
        return Err(AppError::BadRequest(
            "A category, beneficiary name, and deadline are required before campaign submission"
                .into(),
        ));
    }
    let audit_details = json!({
        "title": &campaign.title,
        "category": &campaign.category,
        "beneficiary_name": &campaign.beneficiary_name,
        "submit_for_review": submit_for_review
    });
    let mut tx = state.pool.begin().await?;
    let created = repo::create(&mut tx, user.id, campaign).await?;
    activity_repository::record_audit(
        &mut tx,
        Some(user.id),
        if submit_for_review {
            "campaign.submitted"
        } else {
            "campaign.created"
        },
        "campaign",
        Some(created.id),
        audit_details,
    )
    .await?;
    if submit_for_review {
        activity_repository::notify_role(
            &mut tx,
            "administrator",
            "Campaign needs review",
            &format!("{} submitted a campaign for approval.", user.email),
            Some("/dashboard/administrator#campaign-approvals"),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(created))
}

pub async fn update(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(campaign): Json<UpdateCampaign>,
) -> Result<Json<Campaign>, AppError> {
    if !has_role(&user, &["organizer", "administrator"]) {
        return Err(AppError::Forbidden);
    }
    if campaign
        .title
        .as_ref()
        .is_some_and(|title| title.trim().is_empty() || title.len() > 160)
        || campaign
            .description
            .as_ref()
            .is_some_and(|description| description.trim().is_empty() || description.len() > 10_000)
        || campaign
            .goal_amount
            .is_some_and(|amount| amount <= Decimal::ZERO || amount.fract() != Decimal::ZERO)
        || campaign
            .category
            .as_deref()
            .is_some_and(|value| value.len() > 80)
        || campaign
            .beneficiary_name
            .as_deref()
            .is_some_and(|value| value.len() > 160)
        || campaign
            .beneficiary_details
            .as_deref()
            .is_some_and(|value| value.len() > 4000)
        || campaign
            .supporting_info
            .as_deref()
            .is_some_and(|value| value.len() > 4000)
        || campaign
            .image_url
            .as_deref()
            .is_some_and(|value| value.len() > 2048)
    {
        return Err(AppError::BadRequest(
            "Campaign title and description are required, and goal_amount must be positive".into(),
        ));
    }
    validate_schedule(
        campaign.starts_at.as_ref(),
        campaign.ends_at.as_ref(),
        false,
    )?;

    let submitting = campaign.status.as_deref() == Some("pending_review");
    let audit_details = json!({
        "title": &campaign.title,
        "description": &campaign.description,
        "category": &campaign.category,
        "beneficiary_name": &campaign.beneficiary_name,
        "beneficiary_details": &campaign.beneficiary_details,
        "supporting_info": &campaign.supporting_info,
        "goal_amount": campaign.goal_amount,
        "status": &campaign.status
    });
    let mut tx = state.pool.begin().await?;
    if user.role == "organizer" {
        let current = sqlx::query_as::<_, (
            String,
            Option<String>,
            Option<String>,
            Option<chrono::DateTime<Utc>>,
            Option<chrono::DateTime<Utc>>,
        )>(
            "SELECT status, category, beneficiary_name, starts_at, ends_at FROM campaigns WHERE id = $1 AND organizer_id = $2 FOR UPDATE",
        )
        .bind(id)
        .bind(user.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
        if let Some(status) = campaign.status.as_deref() {
            let allowed = match status {
                "draft" => current.0 == "draft",
                "pending_review" => matches!(
                    current.0.as_str(),
                    "draft" | "rejected" | "changes_requested"
                ),
                _ => false,
            };
            if !allowed {
                return Err(AppError::BadRequest(
                    "Only draft or returned campaigns can be saved or resubmitted".into(),
                ));
            }
            if status == "pending_review" {
                let category = campaign.category.as_deref().or(current.1.as_deref());
                let beneficiary = campaign
                    .beneficiary_name
                    .as_deref()
                    .or(current.2.as_deref());
                let starts_at = campaign.starts_at.as_ref().or(current.3.as_ref());
                let ends_at = campaign.ends_at.as_ref().or(current.4.as_ref());
                if category.is_none_or(str::is_empty)
                    || beneficiary.is_none_or(str::is_empty)
                    || ends_at.is_none()
                {
                    return Err(AppError::BadRequest(
                        "A category, beneficiary name, and deadline are required before campaign submission"
                            .into(),
                    ));
                }
                validate_schedule(starts_at, ends_at, true)?;
            }
        }
    }

    let updated = repo::update(&mut tx, id, user.id, campaign)
        .await?
        .ok_or(AppError::NotFound)?;
    activity_repository::record_audit(
        &mut tx,
        Some(user.id),
        if submitting {
            "campaign.resubmitted"
        } else {
            "campaign.updated"
        },
        "campaign",
        Some(id),
        audit_details,
    )
    .await?;
    if submitting {
        activity_repository::notify_role(
            &mut tx,
            "administrator",
            "Campaign needs review",
            &format!("{} resubmitted a campaign for approval.", user.email),
            Some("/dashboard/administrator#campaign-approvals"),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(updated))
}
