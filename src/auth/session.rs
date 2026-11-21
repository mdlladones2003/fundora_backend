use crate::{app::AppState, auth::jwt, error::AppError};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use rand::{RngCore, rngs::OsRng};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn new_token() -> String {
    let mut b = [0u8; 48];

    OsRng.fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

pub async fn create_session(s: &AppState, uid: Uuid) -> Result<serde_json::Value, AppError> {
    let (access, access_exp) = jwt::issue(uid, &s.config.jwt_secret, &s.config.jwt_issuer)?;
    let refresh = new_token();
    let exp = Utc::now() + Duration::days(30);

    let mut tx = s.pool.begin().await?;
    let active: Option<bool> =
        sqlx::query_scalar("SELECT is_active FROM users WHERE id=$1 FOR UPDATE")
            .bind(uid)
            .fetch_optional(&mut *tx)
            .await?;
    if active != Some(true) {
        return Err(AppError::Unauthorized);
    }

    sqlx::query(
        "
        INSERT INTO user_roles(user_id,role_id)
        SELECT $1,id FROM roles WHERE name='donor'
        AND NOT EXISTS(SELECT 1 FROM user_roles WHERE user_id=$1)
        ON CONFLICT DO NOTHING
    ",
    )
    .bind(uid)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO auth_sessions(user_id,refresh_token_hash,expires_at) VALUES($1,$2,$3)",
    )
    .bind(uid)
    .bind(token_hash(&refresh))
    .bind(exp)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(json!({
        "access_token":access,
        "token_type":"Bearer",
        "expires_at":access_exp,
        "refresh_token":refresh,
        "refresh_expires_at":exp.timestamp(),
        "user_id":uid
    }))
}

pub async fn rotate_session(s: &AppState, token: &str) -> Result<serde_json::Value, AppError> {
    if !(40..=256).contains(&token.len()) {
        return Err(AppError::Unauthorized);
    }

    let mut tx = s.pool.begin().await?;
    let row = sqlx::query(
        "SELECT id,user_id,family_id,revoked_at,replaced_by,expires_at FROM auth_sessions WHERE refresh_token_hash=$1 FOR UPDATE",
    )
    .bind(token_hash(token))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::Unauthorized)?;
    let old: Uuid = row.try_get("id")?;
    let uid: Uuid = row.try_get("user_id")?;
    let family_id: Uuid = row.try_get("family_id")?;
    let revoked_at: Option<chrono::DateTime<Utc>> = row.try_get("revoked_at")?;
    let replaced_by: Option<Uuid> = row.try_get("replaced_by")?;
    let expires_at: chrono::DateTime<Utc> = row.try_get("expires_at")?;

    if revoked_at.is_some() {
        if replaced_by.is_some() {
            sqlx::query(
                "UPDATE auth_sessions SET revoked_at=NOW() WHERE family_id=$1 AND revoked_at IS NULL",
            )
            .bind(family_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
        }
        return Err(AppError::Unauthorized);
    }

    if expires_at <= Utc::now() {
        sqlx::query("UPDATE auth_sessions SET revoked_at=NOW() WHERE id=$1")
            .bind(old)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(AppError::Unauthorized);
    }

    let active: Option<bool> =
        sqlx::query_scalar("SELECT is_active FROM users WHERE id=$1 FOR UPDATE")
            .bind(uid)
            .fetch_optional(&mut *tx)
            .await?;
    if active != Some(true) {
        sqlx::query(
            "UPDATE auth_sessions SET revoked_at=NOW() WHERE user_id=$1 AND revoked_at IS NULL",
        )
        .bind(uid)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Err(AppError::Unauthorized);
    }

    let next = new_token();
    let exp = Utc::now() + Duration::days(30);
    let new_id:Uuid = sqlx::query_scalar("INSERT INTO auth_sessions(user_id,refresh_token_hash,expires_at,family_id) VALUES($1,$2,$3,$4) RETURNING id")
                            .bind(uid)
                            .bind(token_hash(&next))
                            .bind(exp)
                            .bind(family_id)
                            .fetch_one(&mut *tx)
                            .await?;
    let n = sqlx::query("UPDATE auth_sessions SET revoked_at=NOW(),replaced_by=$2,last_used_at=NOW() WHERE id=$1 AND revoked_at IS NULL")
                    .bind(old)
                    .bind(new_id)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();

    if n != 1 {
        return Err(AppError::Unauthorized);
    }

    let (access, access_exp) = jwt::issue(uid, &s.config.jwt_secret, &s.config.jwt_issuer)?;
    tx.commit().await?;

    Ok(json!({
        "access_token":access,
        "token_type":"Bearer",
        "expires_at":access_exp,
        "refresh_token":next,
        "refresh_expires_at":exp.timestamp(),
        "user_id":uid
    }))
}

pub async fn revoke_session(s: &AppState, token: &str) -> Result<(), AppError> {
    if !(40..=256).contains(&token.len()) {
        return Err(AppError::Unauthorized);
    }

    sqlx::query("UPDATE auth_sessions SET revoked_at=NOW(),last_used_at=NOW() WHERE refresh_token_hash=$1 AND revoked_at IS NULL")
        .bind(token_hash(token))
        .execute(&s.pool)
        .await?;

    Ok(())
}
