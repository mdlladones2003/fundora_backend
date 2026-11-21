use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::Value;
use crate::{app::AppState, error::AppError};

#[derive(Deserialize)]
pub struct ProviderTokenRequest {
    pub id_token: String
}

// Validates Google ID token using Google's tokeninfo endpoint and checks audience/email verification.
pub async fn google(State(state): State<AppState>, Json(body): Json<ProviderTokenRequest>) -> Result<Json<Value>, AppError> {
    if state.config.google_client_id.is_empty() {
        return Err(AppError::BadRequest("Google sign-in is not configured".into()));
    }

    let claims: Value = state.http.get("https://oauth2.googleapis.com/tokeninfo")
                                .query(&[("id_token", body.id_token)])
                                .send()
                                .await.map_err(|_| AppError::Unauthorized)?
                                .error_for_status()
                                .map_err(|_| AppError::Unauthorized)?
                                .json()
                                .await.map_err(|_| AppError::Unauthorized)?;

    if claims.get("aud").and_then(Value::as_str) != Some(state.config.google_client_id.as_str())
        || claims.get("email_verified").and_then(Value::as_str) != Some("true") {
        return Err(AppError::Unauthorized);
    }
    
    let sub = claims.get("sub").and_then(Value::as_str).ok_or(AppError::Unauthorized)?;
    let email = claims.get("email").and_then(Value::as_str).ok_or(AppError::Unauthorized)?;
    super::siwe::complete_external_identity(&state, "google", sub, email).await
}

// Fail closed until Apple JWKS signature, issuer, audience, expiry and nonce validation is implemented.
pub async fn apple(State(state): State<AppState>, Json(_body): Json<ProviderTokenRequest>) -> Result<Json<Value>, AppError> {
    if state.config.apple_client_id.is_empty() {
        return Err(AppError::BadRequest("Apple sign-in is not configured".into()));
    }

    Err(AppError::BadRequest("Apple sign-in is disabled until full Apple OIDC/JWKS validation is configured".into()))
}
