use crate::{app::AppState, auth::session, error::AppError};
use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

pub async fn complete_external_identity(
    state: &AppState,
    provider: &str,
    subject: &str,
    email: &str,
) -> Result<Json<Value>, AppError> {
    let email = email.trim().to_lowercase();
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1),hashtext($2))")
        .bind(provider)
        .bind(subject)
        .execute(&mut *tx)
        .await?;

    let existing = sqlx::query(
        "SELECT user_id FROM auth_identities WHERE provider=$1 AND provider_subject=$2",
    )
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

        if found.is_some() {
            return Err(AppError::BadRequest(
                "This sign-in method is not linked to an account. Sign in with an existing method first.".into(),
            ));
        }

        let id: Uuid = sqlx::query_scalar(
            "
            INSERT INTO users(email, display_name, is_active)
            VALUES($1,$2,TRUE) RETURNING id
        ",
        )
        .bind(&email)
        .bind(email.split('@').next().unwrap_or("Fundora User"))
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "
            INSERT INTO auth_identities(user_id, provider, provider_subject, email, email_verified)
            VALUES($1,$2,$3,$4,TRUE)
        ",
        )
        .bind(id)
        .bind(provider)
        .bind(subject)
        .bind(&email)
        .execute(&mut *tx)
        .await?;

        id
    };

    tx.commit().await?;
    Ok(Json(session::create_session(state, uid).await?))
}

pub async fn nonce(
    State(_state): State<AppState>,
    Json(_body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    Err(AppError::BadRequest(
        "MetaMask sign-in is disabled until full SIWE verification is configured".into(),
    ))
}
#[derive(Deserialize)]
pub struct SiweVerifyRequest {
    pub message: String,
    pub signature: String,
}

// Deliberately fail-closed: never authenticate from a client-posted wallet address without signature recovery.
pub async fn verify(
    State(_state): State<AppState>,
    Json(body): Json<SiweVerifyRequest>,
) -> Result<Json<Value>, AppError> {
    if body.message.trim().is_empty() || body.signature.trim().is_empty() {
        return Err(AppError::BadRequest(
            "message and signature are required".into(),
        ));
    }

    Err(AppError::BadRequest("MetaMask verification is disabled until EIP-4361 parsing and EIP-191 signature recovery are installed".into()))
}
