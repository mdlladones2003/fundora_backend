use axum::{
    Extension, Json,
    extract::{Path, State},
};
use chrono::Utc;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::HashMap;
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

#[derive(Debug, Deserialize)]
pub struct TrackCampaignView {
    pub session_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct CampaignDailyStatistic {
    pub date: String,
    pub amount: Decimal,
}

#[derive(Debug, Serialize)]
pub struct CampaignAnalytics {
    pub campaign_id: Uuid,
    pub views_count: i64,
    pub total_donors: i64,
    pub donations_today: Decimal,
    pub daily_statistics: Vec<CampaignDailyStatistic>,
}

#[derive(Debug, FromRow)]
struct CampaignAnalyticsRow {
    campaign_id: Uuid,
    views_count: i64,
    total_donors: i64,
    donations_today: Decimal,
    date: String,
    daily_amount: Decimal,
}

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
               c.title, u.wallet_address AS organizer_wallet_address,
               c.description, c.category, c.beneficiary_name,
               c.beneficiary_details, c.supporting_info, c.image_url,
               c.goal_amount, c.raised_amount, c.currency, c.network,
               c.contract_address, c.status, NULL::text AS review_feedback,
               NULL::uuid AS reviewed_by, NULL::timestamptz AS reviewed_at,
               c.starts_at, c.ends_at, c.created_at, c.updated_at,
               (SELECT COUNT(DISTINCT d.donor_id)::bigint
                FROM donations d
                WHERE d.campaign_id = c.id AND d.status = 'confirmed') AS donor_count
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

pub async fn track_view(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<TrackCampaignView>,
) -> Result<axum::http::StatusCode, AppError> {
    sqlx::query(
        r#"
        INSERT INTO campaign_views(campaign_id, session_id)
        SELECT id, $2
        FROM campaigns
        WHERE id = $1 AND status IN ('published', 'active', 'completed')
        ON CONFLICT (campaign_id, session_id) DO NOTHING
        "#,
    )
    .bind(id)
    .bind(body.session_id)
    .execute(&state.pool)
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub async fn organizer_analytics(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<CampaignAnalytics>>, AppError> {
    if !has_role(&user, &["organizer", "administrator"]) {
        return Err(AppError::Forbidden);
    }

    let rows = sqlx::query_as::<_, CampaignAnalyticsRow>(
        r#"
        WITH days AS (
            SELECT GENERATE_SERIES(
                CURRENT_DATE - INTERVAL '6 days',
                CURRENT_DATE,
                INTERVAL '1 day'
            )::date AS day
        )
        SELECT c.id AS campaign_id,
               COALESCE(views.views_count, 0)::bigint AS views_count,
               COALESCE(donors.total_donors, 0)::bigint AS total_donors,
               COALESCE(today.amount, 0) AS donations_today,
               TO_CHAR(days.day, 'YYYY-MM-DD') AS date,
               COALESCE(daily.amount, 0) AS daily_amount
        FROM campaigns c
        CROSS JOIN days
        LEFT JOIN LATERAL (
            SELECT COUNT(*) AS views_count
            FROM campaign_views v
            WHERE v.campaign_id = c.id
        ) views ON TRUE
        LEFT JOIN LATERAL (
            SELECT COUNT(DISTINCT d.donor_id) AS total_donors
            FROM donations d
            WHERE d.campaign_id = c.id AND d.status = 'confirmed'
        ) donors ON TRUE
        LEFT JOIN LATERAL (
            SELECT SUM(d.amount) AS amount
            FROM donations d
            WHERE d.campaign_id = c.id
              AND d.status = 'confirmed'
              AND COALESCE(d.confirmed_at, d.created_at) >= CURRENT_DATE
              AND COALESCE(d.confirmed_at, d.created_at) < CURRENT_DATE + INTERVAL '1 day'
        ) today ON TRUE
        LEFT JOIN LATERAL (
            SELECT SUM(d.amount) AS amount
            FROM donations d
            WHERE d.campaign_id = c.id
              AND d.status = 'confirmed'
              AND COALESCE(d.confirmed_at, d.created_at) >= days.day
              AND COALESCE(d.confirmed_at, d.created_at) < days.day + INTERVAL '1 day'
        ) daily ON TRUE
        WHERE c.organizer_id = $1
        ORDER BY c.created_at DESC, days.day
        "#,
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;

    let mut campaigns = Vec::<CampaignAnalytics>::new();
    let mut indices = HashMap::<Uuid, usize>::new();
    for row in rows {
        let index = *indices.entry(row.campaign_id).or_insert_with(|| {
            campaigns.push(CampaignAnalytics {
                campaign_id: row.campaign_id,
                views_count: row.views_count,
                total_donors: row.total_donors,
                donations_today: row.donations_today,
                daily_statistics: Vec::with_capacity(7),
            });
            campaigns.len() - 1
        });
        campaigns[index]
            .daily_statistics
            .push(CampaignDailyStatistic {
                date: row.date,
                amount: row.daily_amount,
            });
    }
    Ok(Json(campaigns))
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

fn validate_php_fundraising_goal(
    goal_amount: Decimal,
    currency: Option<&str>,
) -> Result<(), AppError> {
    if goal_amount <= Decimal::ZERO
        || goal_amount.normalize().scale() > 2
        || currency.is_some_and(|value| value != "PHP")
    {
        return Err(AppError::BadRequest(
            "A positive PHP fundraising goal with at most two decimal places is required".into(),
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
    validate_php_fundraising_goal(campaign.goal_amount, campaign.currency.as_deref())?;
    if campaign.title.trim().is_empty()
        || campaign.title.len() > 160
        || campaign.description.trim().is_empty()
        || campaign.description.len() > 10_000
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
            "A title, description, and positive PHP fundraising goal are required".into(),
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
            Some("/admin/campaigns"),
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
            .is_some_and(|amount| amount <= Decimal::ZERO)
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
    if campaign_locked_for_edits(&current.0) {
        return Err(AppError::Conflict(
            "Published campaigns can no longer be edited.".into(),
        ));
    }

    if user.role == "organizer" {
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
            Some("/admin/campaigns"),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(updated))
}

fn campaign_locked_for_edits(status: &str) -> bool {
    matches!(
        status,
        "published" | "active" | "paused" | "completed" | "archived"
    )
}

#[cfg(test)]
mod tests {
    use super::{campaign_locked_for_edits, validate_php_fundraising_goal};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    #[test]
    fn requires_php_goals_with_cent_precision() {
        assert!(
            validate_php_fundraising_goal(Decimal::from_str("1000.50").unwrap(), Some("PHP"))
                .is_ok()
        );
        assert!(validate_php_fundraising_goal(Decimal::from_str("1000").unwrap(), None).is_ok());
        assert!(
            validate_php_fundraising_goal(Decimal::from_str("1000.501").unwrap(), Some("PHP"))
                .is_err()
        );
        assert!(validate_php_fundraising_goal(Decimal::ONE, Some("ETH")).is_err());
    }

    #[test]
    fn locks_campaign_edits_after_publication() {
        for status in ["published", "active", "paused", "completed", "archived"] {
            assert!(campaign_locked_for_edits(status));
        }
        for status in ["draft", "pending_review", "changes_requested", "rejected"] {
            assert!(!campaign_locked_for_edits(status));
        }
    }
}
