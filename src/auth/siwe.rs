use crate::{
    app::AppState,
    auth::{otp, session},
    config::Config,
    error::AppError,
};
use axum::{Json, extract::State};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

struct SiweSettings {
    domain: String,
    uri: String,
    chain_id: u64,
}

fn settings(config: &Config) -> Result<SiweSettings, AppError> {
    let domain = config.siwe_domain.trim();
    let uri = config.siwe_uri.trim();
    let chain_id = config.siwe_chain_id.ok_or(AppError::ServiceUnavailable(
        "Wallet sign-in is not configured. Set SIWE_DOMAIN, SIWE_URI, and SIWE_CHAIN_ID.",
    ))?;
    if domain.is_empty() || uri.is_empty() {
        return Err(AppError::ServiceUnavailable(
            "Wallet sign-in is not configured. Set SIWE_DOMAIN, SIWE_URI, and SIWE_CHAIN_ID.",
        ));
    }

    let parsed = reqwest::Url::parse(uri).map_err(|_| {
        AppError::ServiceUnavailable("Wallet sign-in has an invalid SIWE_URI configuration.")
    })?;
    let host = parsed.host_str().ok_or(AppError::ServiceUnavailable(
        "Wallet sign-in has an invalid SIWE_URI configuration.",
    ))?;
    let expected_domain = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    let local_http = matches!(host, "localhost" | "127.0.0.1" | "::1");
    if (parsed.scheme() != "https" && !(parsed.scheme() == "http" && local_http))
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !domain.eq_ignore_ascii_case(&expected_domain)
    {
        return Err(AppError::ServiceUnavailable(
            "Wallet sign-in domain and URI must match a secure frontend origin.",
        ));
    }

    Ok(SiweSettings {
        domain: domain.to_owned(),
        uri: parsed.as_str().trim_end_matches('/').to_owned(),
        chain_id,
    })
}

fn normalize_wallet_address(address: &str) -> Result<String, AppError> {
    let address = address.trim();
    let hex = address
        .strip_prefix("0x")
        .or_else(|| address.strip_prefix("0X"))
        .ok_or(AppError::Unauthorized)?;
    if hex.len() != 40 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::Unauthorized);
    }
    Ok(format!("0x{}", hex.to_ascii_lowercase()))
}

async fn wallet_account_exists(state: &AppState, address: &str) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM auth_identities WHERE provider='metamask' AND lower(provider_subject)=lower($1)) OR EXISTS(SELECT 1 FROM users WHERE lower(wallet_address)=lower($1))",
    )
    .bind(address)
    .fetch_one(&state.pool)
    .await?)
}

pub async fn complete_external_identity(
    state: &AppState,
    provider: &str,
    subject: &str,
    email: Option<&str>,
) -> Result<Json<Value>, AppError> {
    let email = email.map(|value| value.trim().to_lowercase());
    if email
        .as_ref()
        .is_some_and(|value| !otp::is_valid_email(value))
    {
        return Err(AppError::Unauthorized);
    }
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
    let uid: Uuid = if let Some(row) = existing {
        row.try_get("user_id")?
    } else {
        let email = email.as_deref().ok_or_else(|| {
            AppError::BadRequest("A verified email is required to create this account.".into())
        })?;
        let found = sqlx::query("SELECT id FROM users WHERE lower(email)=$1")
            .bind(email)
            .fetch_optional(&mut *tx)
            .await?;

        if found.is_some() {
            return Err(AppError::BadRequest(
                "This sign-in method is not linked to an account. Sign in with an existing method first.".into(),
            ));
        }

        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO users(email, display_name, is_active) VALUES($1,$2,TRUE) RETURNING id",
        )
        .bind(email)
        .bind(email.split('@').next().unwrap_or("Fundora User"))
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO auth_identities(user_id, provider, provider_subject, email, email_verified) VALUES($1,$2,$3,$4,TRUE)",
        )
        .bind(id)
        .bind(provider)
        .bind(subject)
        .bind(email)
        .execute(&mut *tx)
        .await?;

        id
    };

    tx.commit().await?;
    Ok(Json(session::create_session(state, uid).await?))
}

#[derive(Deserialize)]
pub struct SiweNonceRequest {
    wallet_address: String,
}

pub async fn nonce(
    State(state): State<AppState>,
    Json(body): Json<SiweNonceRequest>,
) -> Result<Json<Value>, AppError> {
    let settings = settings(&state.config)?;
    let wallet = normalize_wallet_address(&body.wallet_address)?;
    let requires_email = !wallet_account_exists(&state, &wallet).await?;
    let nonce = siwe::generate_nonce();
    let expires_at = Utc::now() + Duration::minutes(5);

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE siwe_nonces SET consumed_at=NOW() WHERE lower(wallet_address)=lower($1) AND consumed_at IS NULL",
    )
    .bind(&wallet)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO siwe_nonces(wallet_address,nonce,expires_at) VALUES($1,$2,$3)")
        .bind(&wallet)
        .bind(&nonce)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(Json(json!({
        "nonce": nonce,
        "domain": settings.domain,
        "uri": settings.uri,
        "chain_id": settings.chain_id,
        "statement": "Sign in to Fundora.",
        "issued_at": Utc::now(),
        "expires_at": expires_at,
        "requires_email": requires_email
    })))
}

#[derive(Deserialize)]
pub struct SiweVerifyRequest {
    message: String,
    signature: String,
    email: Option<String>,
}

pub async fn verify(
    State(state): State<AppState>,
    Json(body): Json<SiweVerifyRequest>,
) -> Result<Json<Value>, AppError> {
    let settings = settings(&state.config)?;
    let message: siwe::Message = body.message.parse().map_err(|_| AppError::Unauthorized)?;
    if message.domain.as_str() != settings.domain
        || message.uri.to_string() != settings.uri
        || message.chain_id != settings.chain_id
        || message.statement.as_deref() != Some("Sign in to Fundora.")
        || !message.valid_now()
    {
        return Err(AppError::Unauthorized);
    }

    let signature_hex = body
        .signature
        .strip_prefix("0x")
        .or_else(|| body.signature.strip_prefix("0X"))
        .unwrap_or(&body.signature);
    let signature: [u8; 65] = hex::decode(signature_hex)
        .map_err(|_| AppError::Unauthorized)?
        .try_into()
        .map_err(|_| AppError::Unauthorized)?;
    message
        .verify_eip191(&signature)
        .map_err(|_| AppError::Unauthorized)?;
    let wallet = normalize_wallet_address(&format!("0x{}", hex::encode(message.address)))?;

    let email = body.email.map(|value| value.trim().to_lowercase());
    if email
        .as_ref()
        .is_some_and(|value| !otp::is_valid_email(value))
    {
        return Err(AppError::BadRequest("Enter a valid email address.".into()));
    }

    let mut tx = state.pool.begin().await?;
    let challenge = sqlx::query(
        "SELECT wallet_address FROM siwe_nonces WHERE nonce=$1 AND consumed_at IS NULL AND expires_at>NOW() FOR UPDATE",
    )
    .bind(&message.nonce)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::Unauthorized)?;
    let expected_wallet: String = challenge.try_get("wallet_address")?;
    if expected_wallet.to_lowercase() != wallet {
        return Err(AppError::Unauthorized);
    }

    let existing: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM auth_identities WHERE provider='metamask' AND lower(provider_subject)=$1) OR EXISTS(SELECT 1 FROM users WHERE lower(wallet_address)=$1)",
    )
    .bind(&wallet)
    .fetch_one(&mut *tx)
    .await?;
    if !existing && email.is_none() {
        return Err(AppError::BadRequest(
            "Enter an email address to create an account for this wallet.".into(),
        ));
    }

    sqlx::query("UPDATE siwe_nonces SET consumed_at=NOW() WHERE nonce=$1")
        .bind(&message.nonce)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    complete_wallet_identity(&state, &wallet, email.as_deref()).await
}

async fn complete_wallet_identity(
    state: &AppState,
    wallet: &str,
    email: Option<&str>,
) -> Result<Json<Value>, AppError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('metamask'),hashtext($1))")
        .bind(wallet)
        .execute(&mut *tx)
        .await?;

    let existing = sqlx::query(
        "SELECT user_id FROM auth_identities WHERE provider='metamask' AND provider_subject=$1",
    )
    .bind(wallet)
    .fetch_optional(&mut *tx)
    .await?;
    let uid = if let Some(row) = existing {
        row.try_get("user_id")?
    } else if let Some(row) = sqlx::query("SELECT id FROM users WHERE lower(wallet_address)=$1")
        .bind(wallet)
        .fetch_optional(&mut *tx)
        .await?
    {
        row.try_get("id")?
    } else {
        let email = email.ok_or_else(|| {
            AppError::BadRequest(
                "Enter an email address to create an account for this wallet.".into(),
            )
        })?;
        let email = email.trim().to_lowercase();
        if !otp::is_valid_email(&email) {
            return Err(AppError::BadRequest("Enter a valid email address.".into()));
        }
        let email_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE lower(email)=$1)")
                .bind(&email)
                .fetch_one(&mut *tx)
                .await?;
        if email_exists {
            return Err(AppError::BadRequest(
                "This email belongs to an existing account. Sign in with that method before linking a wallet.".into(),
            ));
        }
        sqlx::query_scalar(
            "INSERT INTO users(email,display_name,wallet_address,is_active) VALUES($1,$2,$3,TRUE) RETURNING id",
        )
        .bind(&email)
        .bind(email.split('@').next().unwrap_or("Wallet user"))
        .bind(wallet)
        .fetch_one(&mut *tx)
        .await?
    };

    sqlx::query(
        "INSERT INTO auth_identities(user_id,provider,provider_subject,email,email_verified) VALUES($1,'metamask',$2,NULL,FALSE) ON CONFLICT(provider,provider_subject) DO NOTHING",
    )
    .bind(uid)
    .bind(wallet)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(session::create_session(state, uid).await?))
}

#[cfg(test)]
mod tests {
    use super::{normalize_wallet_address, settings};
    use crate::config::Config;
    use std::str::FromStr;

    fn test_config() -> Config {
        Config {
            database_url: String::new(),
            port: 8080,
            jwt_secret: String::new(),
            jwt_issuer: String::new(),
            blockchain_webhook_secret: String::new(),
            frontend_origin: String::new(),
            email_from: String::new(),
            resend_api_key: String::new(),
            google_client_id: String::new(),
            apple_client_id: String::new(),
            siwe_domain: "fundora.example".into(),
            siwe_uri: "https://fundora.example".into(),
            siwe_chain_id: Some(1),
        }
    }

    #[test]
    fn normalizes_wallet_addresses_and_rejects_malformed_values() {
        assert_eq!(
            normalize_wallet_address("0xAbCd000000000000000000000000000000000000").unwrap(),
            "0xabcd000000000000000000000000000000000000"
        );
        assert!(normalize_wallet_address("0x1234").is_err());
    }

    #[test]
    fn siwe_settings_require_a_matching_secure_origin() {
        let config = test_config();
        let parsed = settings(&config).unwrap();
        assert_eq!(parsed.domain, "fundora.example");
        assert_eq!(parsed.chain_id, 1);

        let mut invalid_config = test_config();
        invalid_config.siwe_domain = "attacker.example".into();
        assert!(settings(&invalid_config).is_err());
    }

    #[test]
    fn verifies_eip191_message_signatures() {
        let raw_message = "localhost:4361 wants you to sign in with your Ethereum account:\n\
0x6Da01670d8fc844e736095918bbE11fE8D564163\n\
\n\
SIWE Notepad Example\n\
\n\
URI: http://localhost:4361\n\
Version: 1\n\
Chain ID: 1\n\
Nonce: kEWepMt9knR6lWJ6A\n\
Issued At: 2021-12-07T18:28:18.807Z";
        let signature = hex::decode("6228b3ecd7bf2df018183aeab6b6f1db1e9f4e3cbe24560404112e25363540eb679934908143224d746bbb5e1aa65ab435684081f4dbb74a0fec57f98f40f5051c").unwrap();
        let signature: [u8; 65] = signature.try_into().unwrap();
        let message = siwe::Message::from_str(raw_message).unwrap();
        assert!(message.verify_eip191(&signature).is_ok());

        let tampered_message =
            siwe::Message::from_str(&raw_message.replace("Notepad", "Tampered")).unwrap();
        assert!(tampered_message.verify_eip191(&signature).is_err());
    }
}
