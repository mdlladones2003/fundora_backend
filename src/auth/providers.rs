use crate::{app::AppState, error::AppError};
use axum::{Json, extract::State};
use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
pub struct ProviderTokenRequest {
    pub id_token: String,
}

fn expiration(claims: &Value) -> Option<i64> {
    claims
        .get("exp")
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
}

fn email_is_verified(claims: &Value) -> bool {
    match claims.get("email_verified") {
        Some(Value::Bool(true)) => true,
        Some(Value::String(value)) => value == "true",
        _ => false,
    }
}

// Validates Google ID token using Google's tokeninfo endpoint and checks audience/email verification.
pub async fn google(
    State(state): State<AppState>,
    Json(body): Json<ProviderTokenRequest>,
) -> Result<Json<Value>, AppError> {
    if state.config.google_client_id.is_empty() {
        return Err(AppError::BadRequest(
            "Google sign-in is not configured".into(),
        ));
    }

    let claims: Value = state
        .http
        .get("https://oauth2.googleapis.com/tokeninfo")
        .query(&[("id_token", body.id_token)])
        .send()
        .await
        .map_err(|_| AppError::Unauthorized)?
        .error_for_status()
        .map_err(|_| AppError::Unauthorized)?
        .json()
        .await
        .map_err(|_| AppError::Unauthorized)?;

    let issuer = claims.get("iss").and_then(Value::as_str);
    if claims.get("aud").and_then(Value::as_str) != Some(state.config.google_client_id.as_str())
        || !matches!(
            issuer,
            Some("accounts.google.com" | "https://accounts.google.com")
        )
        || expiration(&claims).is_none_or(|value| value <= Utc::now().timestamp())
        || !email_is_verified(&claims)
    {
        return Err(AppError::Unauthorized);
    }

    let sub = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|subject| !subject.is_empty())
        .ok_or(AppError::Unauthorized)?;
    let email = claims
        .get("email")
        .and_then(Value::as_str)
        .ok_or(AppError::Unauthorized)?;
    super::siwe::complete_external_identity(&state, "google", sub, email).await
}

// Fail closed until Apple JWKS signature, issuer, audience, expiry and nonce validation is implemented.
pub async fn apple(
    State(state): State<AppState>,
    Json(_body): Json<ProviderTokenRequest>,
) -> Result<Json<Value>, AppError> {
    if state.config.apple_client_id.is_empty() {
        return Err(AppError::BadRequest(
            "Apple sign-in is not configured".into(),
        ));
    }

    Err(AppError::BadRequest(
        "Apple sign-in is disabled until full Apple OIDC/JWKS validation is configured".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{email_is_verified, expiration};
    use serde_json::json;

    #[test]
    fn google_expiration_accepts_numeric_and_string_claims() {
        assert_eq!(
            expiration(&json!({"exp": 1_800_000_000})),
            Some(1_800_000_000)
        );
        assert_eq!(
            expiration(&json!({"exp": "1800000000"})),
            Some(1_800_000_000)
        );
    }

    #[test]
    fn google_email_must_be_verified() {
        assert!(email_is_verified(&json!({"email_verified": true})));
        assert!(email_is_verified(&json!({"email_verified": "true"})));
        assert!(!email_is_verified(&json!({"email_verified": false})));
    }
}
