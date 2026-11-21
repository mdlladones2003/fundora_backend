use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Duration, Utc};
use rand::{rngs::OsRng, RngCore};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;
use crate::{
    app::AppState,
    auth::jwt,
    error::AppError
};

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn new_token() -> String {
    let mut b = [0u8;48];
    
    OsRng.fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

pub async fn create_session(s:&AppState, uid:Uuid)->Result<serde_json::Value,AppError>{
    let (access, access_exp) = jwt::issue(uid, &s.config.jwt_secret, &s.config.jwt_issuer)?;
    let refresh = new_token();
    let exp = Utc::now() + Duration::days(30);
    
    sqlx::query("INSERT INTO auth_sessions(user_id,refresh_token_hash,expires_at) VALUES($1,$2,$3)")
        .bind(uid)
        .bind(token_hash(&refresh))
        .bind(exp)
        .execute(&s.pool)
        .await?;

    Ok(json!({
        "access_token":access,
        "token_type":"Bearer",
        "expires_at":access_exp,
        "refresh_token":refresh,
        "refresh_expires_at":exp.timestamp(),
        "user_id":uid
    }))
}

pub async fn rotate_session(s:&AppState, token:&str) -> Result<serde_json::Value, AppError>{
    if !(40..=256).contains(&token.len()) {
        return Err(AppError::Unauthorized)
    }

    let mut tx = s.pool.begin().await?;
    let row = sqlx::query("SELECT id,user_id FROM auth_sessions WHERE refresh_token_hash=$1 AND revoked_at IS NULL AND expires_at>NOW() FOR UPDATE")
                    .bind(token_hash(token))
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or(AppError::Unauthorized)?;
    let old:Uuid = row.try_get("id")?;
    let uid:Uuid = row.try_get("user_id")?;
    let next = new_token();
    let exp = Utc::now() + Duration::days(30);
    let new_id:Uuid = sqlx::query_scalar("INSERT INTO auth_sessions(user_id,refresh_token_hash,expires_at) VALUES($1,$2,$3) RETURNING id")
                            .bind(uid)
                            .bind(token_hash(&next))
                            .bind(exp)
                            .fetch_one(&mut *tx)
                            .await?;
    let n = sqlx::query("UPDATE auth_sessions SET revoked_at=NOW(),replaced_by=$2,last_used_at=NOW() WHERE id=$1 AND revoked_at IS NULL")
                    .bind(old)
                    .bind(new_id)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
    
    if n != 1 {
        return Err(AppError::Unauthorized)
    }
    
    tx.commit().await?;

    let (access, access_exp) = jwt::issue(uid, &s.config.jwt_secret, &s.config.jwt_issuer)?;

    Ok(json!({
        "access_token":access,
        "token_type":"Bearer",
        "expires_at":access_exp,
        "refresh_token":next,
        "refresh_expires_at":exp.timestamp(),
        "user_id":uid
    }))
}

pub async fn revoke_session(s:&AppState, token:&str) -> Result<(), AppError>{
    if !(40..=256).contains(&token.len()) {
        return Err(AppError::Unauthorized)
    }

    sqlx::query("UPDATE auth_sessions SET revoked_at=NOW(),last_used_at=NOW() WHERE refresh_token_hash=$1 AND revoked_at IS NULL")
        .bind(token_hash(token))
        .execute(&s.pool)
        .await?;
    
    Ok(())
}
