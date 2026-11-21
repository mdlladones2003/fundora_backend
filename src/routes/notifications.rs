use axum::{
    Json,
    extract::{Extension, Path, State},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{app::AppState, auth::AuthUser, error::AppError};

#[derive(Debug, Serialize, FromRow)]
pub struct UserNotification {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub href: Option<String>,
    pub created_at: DateTime<Utc>,
    pub read_at: Option<DateTime<Utc>>,
}

pub async fn list_mine(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<UserNotification>>, AppError> {
    let notifications = sqlx::query_as::<_, UserNotification>(
        r#"
        SELECT id, title, body, href, created_at, read_at
        FROM notifications
        WHERE user_id = $1
        ORDER BY created_at DESC
        LIMIT 100
        "#,
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(notifications))
}

pub async fn mark_read(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<UserNotification>, AppError> {
    let notification = sqlx::query_as::<_, UserNotification>(
        r#"
        UPDATE notifications
        SET read_at = COALESCE(read_at, NOW())
        WHERE id = $1 AND user_id = $2
        RETURNING id, title, body, href, created_at, read_at
        "#,
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(notification))
}
