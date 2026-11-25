use crate::{app::AppState, error::AppError};
use axum::{Json, extract::State};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::Duration;

#[derive(Deserialize)]
pub struct ProviderTokenRequest {
    pub id_token: String,
    pub nonce: String,
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

fn apple_email_is_verified(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(true)) => true,
        Some(Value::String(value)) => value == "true",
        _ => false,
    }
}

fn apple_nonce_matches(claim: &str, nonce: &str) -> bool {
    let digest = Sha256::digest(nonce.as_bytes());
    claim == nonce || claim == hex::encode(digest) || claim == URL_SAFE_NO_PAD.encode(digest)
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
        .query(&[("id_token", body.id_token.as_str())])
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
        || claims.get("nonce").and_then(Value::as_str) != Some(body.nonce.as_str())
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
    super::siwe::complete_external_identity(&state, "google", sub, Some(email)).await
}

#[derive(Deserialize)]
pub struct AppleTokenRequest {
    pub id_token: String,
    pub nonce: String,
}

#[derive(Deserialize)]
struct AppleClaims {
    sub: String,
    email: Option<String>,
    email_verified: Option<Value>,
    nonce: Option<String>,
}

pub async fn apple(
    State(state): State<AppState>,
    Json(body): Json<AppleTokenRequest>,
) -> Result<Json<Value>, AppError> {
    if state.config.apple_client_id.trim().is_empty() {
        return Err(AppError::ServiceUnavailable(
            "Apple sign-in is not configured. Set APPLE_CLIENT_ID.",
        ));
    }

    let header = decode_header(&body.id_token).map_err(|_| AppError::Unauthorized)?;
    if header.alg != Algorithm::RS256 {
        return Err(AppError::Unauthorized);
    }
    let kid = header.kid.as_deref().ok_or(AppError::Unauthorized)?;
    let response = state
        .http
        .get("https://appleid.apple.com/auth/keys")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|error| {
            tracing::error!("Failed to fetch Apple signing keys: {error}");
            AppError::ServiceUnavailable("Apple sign-in is temporarily unavailable.")
        })?;
    if !response.status().is_success() {
        tracing::error!("Apple signing-key endpoint returned {}", response.status());
        return Err(AppError::ServiceUnavailable(
            "Apple sign-in is temporarily unavailable.",
        ));
    }
    let keys: JwkSet = response.json().await.map_err(|error| {
        tracing::error!("Apple signing-key response was invalid: {error}");
        AppError::ServiceUnavailable("Apple sign-in is temporarily unavailable.")
    })?;
    let jwk = keys.find(kid).ok_or(AppError::Unauthorized)?;
    let key = DecodingKey::from_jwk(jwk).map_err(|_| AppError::Unauthorized)?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&["https://appleid.apple.com"]);
    validation.set_audience(&[state.config.apple_client_id.trim()]);
    validation.set_required_spec_claims(&["iss", "aud", "exp", "sub"]);
    let claims = decode::<AppleClaims>(&body.id_token, &key, &validation)
        .map_err(|_| AppError::Unauthorized)?
        .claims;

    if claims.sub.is_empty()
        || !claims
            .nonce
            .as_deref()
            .is_some_and(|nonce| apple_nonce_matches(nonce, &body.nonce))
        || claims
            .email
            .as_ref()
            .is_some_and(|_| !apple_email_is_verified(claims.email_verified.as_ref()))
    {
        return Err(AppError::Unauthorized);
    }
    super::siwe::complete_external_identity(&state, "apple", &claims.sub, claims.email.as_deref())
        .await
}

#[cfg(test)]
mod tests {
    use super::{apple_email_is_verified, apple_nonce_matches, email_is_verified, expiration};
    use serde_json::json;
    use sha2::Digest;

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

    #[test]
    fn apple_email_must_be_verified_when_present() {
        assert!(apple_email_is_verified(Some(&json!(true))));
        assert!(apple_email_is_verified(Some(&json!("true"))));
        assert!(!apple_email_is_verified(Some(&json!(false))));
        assert!(!apple_email_is_verified(None));
    }

    #[test]
    fn apple_nonce_accepts_raw_and_sha256_forms_only() {
        assert!(apple_nonce_matches("request-nonce", "request-nonce"));
        let digest = sha2::Sha256::digest(b"request-nonce");
        assert!(apple_nonce_matches(&hex::encode(digest), "request-nonce"));
        assert!(!apple_nonce_matches("different-nonce", "request-nonce"));
    }
}
