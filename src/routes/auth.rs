use axum::{extract::State, Json};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use crate::{
    app::AppState,
    auth::{otp,session},
    database::repositories::user_repository,
    error::AppError
};

#[derive(Deserialize)]
pub struct EmailRequest {
    pub email: String
}

fn is_valid_email(email: &str) -> bool {
    if email.is_empty() || email.len() > 254 {
        return false;
    }
    match email.split_once('@') {
        Some((local, domain)) => !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.'),
        None => false,
    }
}

pub async fn request_email_code(
    State(s): State<AppState>,
    Json(b): Json<EmailRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let email = b.email.trim().to_lowercase();
    if !is_valid_email(&email)
        || s.config.resend_api_key.trim().is_empty()
        || s.config.email_from.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "Valid email and email provider configuration are required".into(),
        ));
    }

    let pepper = std::env::var("OTP_PEPPER").map_err(|e| {
        tracing::error!("OTP_PEPPER is missing or invalid: {e}");
        AppError::Internal
    })?;

    let recent: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM email_otps WHERE email=$1 AND created_at>NOW()-INTERVAL '60 seconds')",
    )
    .bind(&email)
    .fetch_one(&s.pool)
    .await?;

    if recent {
        return Err(AppError::BadRequest(
            "Please wait before requesting another code".into(),
        ));
    }

    let code = otp::generate_code();
    let hash = otp::hash_code(&email, &code, &pepper);
    let exp = Utc::now() + Duration::minutes(8);

    let payload = json!({
        "from": s.config.email_from.trim(),
        "to": [email.as_str()],
        "subject": "Your Fundora sign-in code",
        "html": format!(
            "<p>Your Fundora verification code:</p><h2>{}</h2><p>Expires in 8 minutes. Do not share it.</p>",
            code
        )
    });

    let response = s
        .http
        .post("https://api.resend.com/emails")
        .bearer_auth(s.config.resend_api_key.trim())
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Failed to send request to Resend: {e}");
            AppError::Internal
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let err_body = response.text().await.unwrap_or_default();
        tracing::warn!("Resend rejected email ({status}): {err_body}");
        return Err(AppError::Internal);
    }

    let mut tx = s.pool.begin().await?;
    sqlx::query("UPDATE email_otps SET consumed_at=NOW() WHERE email=$1 AND consumed_at IS NULL")
        .bind(&email)
        .execute(&mut *tx)
        .await?;

    sqlx::query("INSERT INTO email_otps(email,code_hash,expires_at) VALUES($1,$2,$3)")
        .bind(&email)
        .bind(hash)
        .bind(exp)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(Json(json!({
        "message": "If the address is eligible, a sign-in code has been sent."
    })))
}

#[derive(Deserialize)]
pub struct VerifyEmailRequest {
    pub email:String,
    pub code:String
}

pub async fn verify_email_code(
    State(s): State<AppState>,
    Json(b): Json<VerifyEmailRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let email = b.email.trim().to_lowercase();
    let code = b.code.trim();
    if !is_valid_email(&email) || code.is_empty() {
        return Err(AppError::Unauthorized);
    }

    let pepper = std::env::var("OTP_PEPPER").map_err(|e| {
        tracing::error!("OTP_PEPPER is missing or invalid: {e}");
        AppError::Internal
    })?;

    let hash = otp::hash_code(&email, code, &pepper);
    let mut tx = s.pool.begin().await?;

    let row = sqlx::query(
        "SELECT id,code_hash FROM email_otps WHERE email=$1 AND consumed_at IS NULL AND expires_at>NOW() AND attempts<5 ORDER BY created_at DESC LIMIT 1 FOR UPDATE",
    )
    .bind(&email)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::Unauthorized)?;

    let otp_id: uuid::Uuid = row.try_get("id")?;
    let stored: String = row.try_get("code_hash")?;

    if stored.trim() != hash {
        sqlx::query("UPDATE email_otps SET attempts=attempts+1 WHERE id=$1")
            .bind(otp_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(AppError::Unauthorized);
    }

    sqlx::query("UPDATE email_otps SET consumed_at=NOW() WHERE id=$1")
        .bind(otp_id)
        .execute(&mut *tx)
        .await?;

    let found = sqlx::query("SELECT id FROM users WHERE lower(email)=$1")
        .bind(&email)
        .fetch_optional(&mut *tx)
        .await?;

    let uid: uuid::Uuid = if let Some(r) = found {
        r.try_get("id")?
    } else {
        let local_part = email.split('@').next().unwrap_or("");
        let display_name = if local_part.is_empty() {
            "Fundora User"
        } else {
            local_part
        };
        sqlx::query_scalar(
            "INSERT INTO users(email,display_name,is_active) VALUES($1,$2,TRUE) RETURNING id",
        )
        .bind(&email)
        .bind(display_name)
        .fetch_one(&mut *tx)
        .await?
    };

    sqlx::query(
        "INSERT INTO auth_identities(user_id,provider,provider_subject,email,email_verified) VALUES($1,'email',$2,$2,TRUE) ON CONFLICT(provider,provider_subject) DO NOTHING",
    )
    .bind(uid)
    .bind(&email)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let result = session::create_session(&s, uid).await?;
    Ok(Json(result))
}

#[derive(Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String
}

pub async fn refresh(State(state): State<AppState>, Json(body): Json<RefreshRequest>)
    -> Result<Json<serde_json::Value>, AppError> {
    Ok(Json(session::rotate_session(&state, &body.refresh_token).await?))
}

pub async fn logout(State(state): State<AppState>, Json(body): Json<RefreshRequest>)
    -> Result<Json<serde_json::Value>, AppError> {
    session::revoke_session(&state, &body.refresh_token).await?;
    Ok(Json(json!({"message":"Session revoked"})))
}

pub async fn me(
    State(state): State<AppState>,
    axum::Extension(claims): axum::Extension<crate::auth::jwt::Claims>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user = user_repository::get(&state.pool, claims.sub)
        .await?
        .ok_or(AppError::Unauthorized)?;
    Ok(Json(json!({
        "id": user.id,
        "email": user.email,
        "display_name": user.display_name,
        "avatar_url": user.avatar_url,
        "wallet_address": user.wallet_address,
        "created_at": user.created_at
    })))
}
