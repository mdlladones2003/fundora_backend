use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use crate::database::models::role::Role;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub id: uuid::Uuid,
    pub email: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Claims {
    sub: String,
    email: String,
    role: String,
    exp: usize,
}

impl AuthUser {
    pub fn from_token(token: &str, secret: &str) -> Result<Self, StatusCode> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

        let claims = token_data.claims;

        Ok(Self {
            id: claims
                .sub
                .parse()
                .map_err(|_| StatusCode::UNAUTHORIZED)?,
            email: claims.email,
            role: claims.role,
        })
    }
}

pub fn has_role(user: &AuthUser, required_roles: &[&str]) -> bool {
    Role::from_name(&user.role)
        .is_some_and(|role| required_roles.contains(&role.as_str()))
}

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let secret = std::env::var("JWT_SECRET")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        Self::from_token(token, &secret)
    }
}