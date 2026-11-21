use axum::{
    Json,
    extract::{Extension, State},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::{AuthUser, has_role},
    database::repositories::activity_repository,
    error::AppError,
};

#[derive(Debug, Serialize, FromRow)]
pub struct RoleRequest {
    pub id: Uuid,
    pub user_id: Uuid,
    pub requested_role: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub reviewed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct AdminRoleRequest {
    pub id: Uuid,
    pub user_id: Uuid,
    pub email: String,
    pub display_name: Option<String>,
    pub requested_role: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub reviewed_at: Option<DateTime<Utc>>,
}

pub async fn request_organizer(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<RoleRequest>, AppError> {
    if user.role != "donor" {
        return Err(AppError::Forbidden);
    }

    let mut tx = state.pool.begin().await?;
    let request = sqlx::query_as::<_, RoleRequest>(
        r#"
        INSERT INTO role_requests(user_id, requested_role)
        VALUES ($1, 'organizer')
        ON CONFLICT (user_id) WHERE status = 'pending'
        DO UPDATE SET created_at = role_requests.created_at
        RETURNING id, user_id, requested_role, status, created_at, reviewed_at
        "#,
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;

    activity_repository::notify_role(
        &mut tx,
        "administrator",
        "Organizer access request",
        &format!("{} requested organizer access.", user.email),
        Some("/dashboard/administrator#admin-users"),
    )
    .await?;
    activity_repository::record_audit(
        &mut tx,
        Some(user.id),
        "organizer_access.requested",
        "role_request",
        Some(request.id),
        json!({"requested_role": &request.requested_role}),
    )
    .await?;
    tx.commit().await?;

    Ok(Json(request))
}

pub async fn get_mine(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Option<RoleRequest>>, AppError> {
    let request = sqlx::query_as::<_, RoleRequest>(
        r#"
        SELECT id, user_id, requested_role, status, created_at, reviewed_at
        FROM role_requests
        WHERE user_id = $1
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(user.id)
    .fetch_optional(&state.pool)
    .await?;

    Ok(Json(request))
}

pub async fn list_pending(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<AdminRoleRequest>>, AppError> {
    if !has_role(&user, &["administrator"]) {
        return Err(AppError::Forbidden);
    }

    let requests = sqlx::query_as::<_, AdminRoleRequest>(
        r#"
        SELECT rr.id, rr.user_id, u.email, u.display_name, rr.requested_role, rr.status, rr.created_at, rr.reviewed_at
        FROM role_requests rr
        JOIN users u ON u.id = rr.user_id
        WHERE rr.status = 'pending'
        ORDER BY rr.created_at ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(requests))
}

#[derive(serde::Deserialize)]
pub struct ReviewRoleRequest {
    pub decision: String,
}

pub async fn review(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthUser>,
    axum::extract::Path(request_id): axum::extract::Path<Uuid>,
    Json(body): Json<ReviewRoleRequest>,
) -> Result<Json<RoleRequest>, AppError> {
    if !has_role(&admin, &["administrator"]) {
        return Err(AppError::Forbidden);
    }
    if !matches!(body.decision.as_str(), "approved" | "rejected") {
        return Err(AppError::BadRequest(
            "Decision must be approved or rejected".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    let request = sqlx::query_as::<_, RoleRequest>(
        r#"
        SELECT id, user_id, requested_role, status, created_at, reviewed_at
        FROM role_requests
        WHERE id = $1 AND status = 'pending'
        FOR UPDATE
        "#,
    )
    .bind(request_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;

    if body.decision == "approved" {
        sqlx::query("LOCK TABLE user_roles IN EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await?;
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id = $1 FOR UPDATE")
            .bind(request.user_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AppError::NotFound)?;

        let current_role = sqlx::query_scalar::<_, String>(
            r#"
            SELECT r.name
            FROM user_roles ur
            JOIN roles r ON r.id = ur.role_id
            WHERE ur.user_id = $1
            ORDER BY CASE r.name WHEN 'administrator' THEN 3 WHEN 'organizer' THEN 2 WHEN 'donor' THEN 1 ELSE 0 END DESC
            LIMIT 1
            "#,
        )
        .bind(request.user_id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or_else(|| "donor".into());

        if current_role != "donor" {
            return Err(AppError::BadRequest(
                "Only a donor can be approved for organizer access".into(),
            ));
        }

        sqlx::query("DELETE FROM user_roles WHERE user_id = $1")
            .bind(request.user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO user_roles(user_id, role_id) SELECT $1, id FROM roles WHERE name = 'organizer'",
        )
        .bind(request.user_id)
        .execute(&mut *tx)
        .await?;
    }

    let reviewed = sqlx::query_as::<_, RoleRequest>(
        r#"
        UPDATE role_requests
        SET status = $2, reviewed_at = NOW(), reviewed_by = $3
        WHERE id = $1
        RETURNING id, user_id, requested_role, status, created_at, reviewed_at
        "#,
    )
    .bind(request_id)
    .bind(&body.decision)
    .bind(admin.id)
    .fetch_one(&mut *tx)
    .await?;

    activity_repository::record_audit(
        &mut tx,
        Some(admin.id),
        "organizer_access.reviewed",
        "role_request",
        Some(request_id),
        json!({"decision": body.decision, "user_id": request.user_id}),
    )
    .await?;
    activity_repository::notify_user(
        &mut tx,
        request.user_id,
        if body.decision == "approved" {
            "Organizer access approved"
        } else {
            "Organizer access request declined"
        },
        if body.decision == "approved" {
            "Your account now has access to organizer tools."
        } else {
            "Your account remains a donor. You can submit another request later."
        },
        Some("/dashboard"),
    )
    .await?;

    tx.commit().await?;
    Ok(Json(reviewed))
}
