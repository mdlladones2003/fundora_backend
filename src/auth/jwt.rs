use crate::error::AppError;
use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub iss: String,
    pub iat: usize,
    pub exp: usize,
}

pub fn issue(user_id: Uuid, secret: &str, issuer: &str) -> Result<(String, i64), AppError> {
    let now = Utc::now();
    let exp = now + Duration::minutes(15);
    let claims = Claims {
        sub: user_id,
        iss: issuer.into(),
        iat: now.timestamp() as usize,
        exp: exp.timestamp() as usize,
    };
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|_| AppError::Internal)?;

    Ok((token, exp.timestamp()))
}
pub fn verify(token: &str, secret: &str, issuer: &str) -> Result<Claims, AppError> {
    let mut v = Validation::default();

    v.set_issuer(&[issuer]);

    decode::<Claims>(token, &DecodingKey::from_secret(secret.as_bytes()), &v)
        .map(|d| d.claims)
        .map_err(|_| AppError::Unauthorized)
}
