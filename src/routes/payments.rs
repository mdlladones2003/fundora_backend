use crate::{
    app::AppState,
    auth::{AuthUser, has_role},
    config::PaymentMode,
    database::repositories::activity_repository,
    error::AppError,
};
use axum::{
    Json,
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;
use std::{str::FromStr, time::Duration};
use uuid::Uuid;

const XENDIT_API_URL: &str = "https://api.xendit.co";

#[derive(Debug, Deserialize)]
pub struct CreatePaymentRequest {
    pub campaign_id: Uuid,
    pub amount: Decimal,
    pub payment_method: String,
    pub is_anonymous: Option<bool>,
    pub message: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct PaymentStatus {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub amount: Decimal,
    pub currency: String,
    pub status: String,
    pub payment_method: String,
    pub checkout_url: Option<String>,
    pub test_mode: bool,
}

#[derive(Debug, Deserialize)]
struct XenditSession {
    payment_session_id: String,
    reference_id: String,
    status: String,
    amount: serde_json::Number,
    currency: String,
    allowed_payment_channels: Option<Vec<String>>,
    payment_link_url: Option<String>,
}

fn normalize_payment_method(value: &str) -> Result<&'static str, AppError> {
    match value.trim().to_ascii_uppercase().as_str() {
        "GCASH" => Ok("GCASH"),
        "MAYA" | "PAYMAYA" => Ok("MAYA"),
        _ => Err(AppError::BadRequest(
            "Choose GCash or Maya as the payment method.".into(),
        )),
    }
}

fn xendit_channel(method: &str) -> &'static str {
    if method == "GCASH" {
        "GCASH"
    } else {
        "PAYMAYA"
    }
}

fn session_channels_match(channels: Option<&[String]>, method: &str) -> bool {
    channels.is_none_or(|channels| channels.len() == 1 && channels[0] == xendit_channel(method))
}

fn valid_checkout_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && matches!(
                url.host_str(),
                Some("checkout.xendit.co" | "checkout-staging.xendit.co" | "xen.to" | "dev.xen.to")
            )
    })
}

fn validate_contribution_amount(amount: Decimal) -> Result<(), AppError> {
    if amount < Decimal::ONE || amount > Decimal::from(100_000) || amount.scale() > 2 {
        return Err(AppError::BadRequest(
            "Contribution must be PHP 1 to 100,000 with at most two decimal places.".into(),
        ));
    }
    Ok(())
}

fn test_mode(state: &AppState) -> bool {
    match state.config.payment_mode {
        PaymentMode::Mock => true,
        PaymentMode::Xendit => state
            .config
            .xendit_secret_key
            .as_deref()
            .is_some_and(|key| key.starts_with("xnd_development_")),
        PaymentMode::Disabled => false,
    }
}

fn build_return_url(
    origin: &str,
    campaign_id: Uuid,
    donation_id: Uuid,
    canceled: bool,
) -> Option<String> {
    let mut url = reqwest::Url::parse(origin).ok()?;
    if url.scheme() != "https" {
        return None;
    }
    let path = if canceled {
        format!("/campaigns/{campaign_id}")
    } else {
        "/dashboard".to_owned()
    };
    url.set_path(&path);
    url.query_pairs_mut().append_pair(
        if canceled {
            "paymentCanceled"
        } else {
            "paymentDonation"
        },
        &donation_id.to_string(),
    );
    Some(url.into())
}

pub async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<CreatePaymentRequest>,
) -> Result<Json<PaymentStatus>, AppError> {
    if !has_role(&user, &["donor"]) {
        return Err(AppError::Forbidden);
    }
    if state.config.payment_mode == PaymentMode::Disabled {
        return Err(AppError::ServiceUnavailable(
            "GCash and Maya contributions are not configured.",
        ));
    }
    let method = normalize_payment_method(&request.payment_method)?;
    validate_contribution_amount(request.amount)?;
    if request
        .message
        .as_deref()
        .is_some_and(|message| message.len() > 500)
    {
        return Err(AppError::BadRequest(
            "Contribution messages are limited to 500 characters.".into(),
        ));
    }

    let (campaign_title, currency) = sqlx::query_as::<_, (String, String)>(
        "SELECT title, currency FROM campaigns WHERE id=$1 AND status IN ('published','active')",
    )
    .bind(request.campaign_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    if currency != "PHP" {
        return Err(AppError::Conflict(
            "GCash and Maya contributions require a PHP-denominated campaign.".into(),
        ));
    }

    let donation_id = Uuid::new_v4();
    let reference_id = donation_id.to_string();
    let provider = match state.config.payment_mode {
        PaymentMode::Mock => "mock",
        PaymentMode::Xendit => "xendit",
        PaymentMode::Disabled => return Err(AppError::Internal),
    };
    let mut transaction = state.pool.begin().await?;
    sqlx::query(
        r#"
        INSERT INTO donations(
            id, campaign_id, donor_id, amount, currency, status, is_anonymous,
            message, payment_amount, payment_currency
        )
        VALUES($1, $2, $3, $4, 'PHP', 'pending', COALESCE($5, false), $6, $4, 'PHP')
        "#,
    )
    .bind(donation_id)
    .bind(request.campaign_id)
    .bind(user.id)
    .bind(request.amount)
    .bind(request.is_anonymous)
    .bind(request.message)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO payment_attempts(donation_id, provider, method, reference_id)
        VALUES($1, $2, $3, $4)
        "#,
    )
    .bind(donation_id)
    .bind(provider)
    .bind(method)
    .bind(&reference_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    if provider == "xendit" {
        let session = match create_xendit_session(
            &state,
            donation_id,
            request.campaign_id,
            request.amount,
            method,
            &campaign_title,
        )
        .await
        {
            Ok(session) => session,
            Err(error) => {
                mark_payment_failed(&state, donation_id).await?;
                return Err(error);
            }
        };
        let session_amount = Decimal::from_str(&session.amount.to_string()).ok();
        let Some(checkout_url) = session
            .payment_link_url
            .filter(|url| valid_checkout_url(url))
            .filter(|_| {
                session.reference_id == reference_id
                    && session_amount == Some(request.amount)
                    && session.currency == "PHP"
                    && session.status == "ACTIVE"
                    && session_channels_match(session.allowed_payment_channels.as_deref(), method)
                    && valid_session_id(&session.payment_session_id)
            })
        else {
            mark_payment_failed(&state, donation_id).await?;
            return Err(AppError::ServiceUnavailable(
                "Xendit returned an invalid hosted checkout session.",
            ));
        };
        sqlx::query(
            r#"
            UPDATE payment_attempts
            SET session_id=$2, checkout_url=$3, updated_at=now()
            WHERE donation_id=$1 AND status='pending'
            "#,
        )
        .bind(donation_id)
        .bind(session.payment_session_id)
        .bind(&checkout_url)
        .execute(&state.pool)
        .await?;
        return Ok(Json(PaymentStatus {
            id: donation_id,
            campaign_id: request.campaign_id,
            amount: request.amount,
            currency,
            status: "pending".into(),
            payment_method: method.into(),
            checkout_url: Some(checkout_url),
            test_mode: test_mode(&state),
        }));
    }

    Ok(Json(PaymentStatus {
        id: donation_id,
        campaign_id: request.campaign_id,
        amount: request.amount,
        currency,
        status: "pending".into(),
        payment_method: method.into(),
        checkout_url: None,
        test_mode: true,
    }))
}

async fn create_xendit_session(
    state: &AppState,
    donation_id: Uuid,
    campaign_id: Uuid,
    amount: Decimal,
    method: &str,
    campaign_title: &str,
) -> Result<XenditSession, AppError> {
    let api_key = state
        .config
        .xendit_secret_key
        .as_deref()
        .ok_or(AppError::ServiceUnavailable(
            "XENDIT_SECRET_KEY is not configured.",
        ))?;
    let amount = serde_json::Number::from_str(&amount.normalize().to_string())
        .map_err(|_| AppError::Internal)?;
    let allowed_payment_channel = xendit_channel(method);
    let mut body = json!({
        "reference_id": donation_id.to_string(),
        "session_type": "PAY",
        "mode": "PAYMENT_LINK",
        "allow_save_payment_method": "DISABLED",
        "amount": amount,
        "currency": "PHP",
        "country": "PH",
        "allowed_payment_channels": [allowed_payment_channel],
        "description": format!("Fundora contribution: {campaign_title}"),
        "locale": "en"
    });
    if let Some(url) = build_return_url(
        &state.config.frontend_origin,
        campaign_id,
        donation_id,
        false,
    ) {
        body["success_return_url"] = Value::String(url);
    }
    if let Some(url) = build_return_url(
        &state.config.frontend_origin,
        campaign_id,
        donation_id,
        true,
    ) {
        body["cancel_return_url"] = Value::String(url);
    }

    let response = state
        .http
        .post(format!("{XENDIT_API_URL}/sessions"))
        .basic_auth(api_key, Some(""))
        .timeout(Duration::from_secs(15))
        .json(&body)
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(%error, "Could not create Xendit payment session");
            AppError::ServiceUnavailable("Xendit checkout is temporarily unavailable.")
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let provider_error = response.json::<Value>().await.ok();
        let error_code = provider_error
            .as_ref()
            .and_then(|payload| payload.get("error_code"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        tracing::warn!(
            %status,
            error_code,
            "Xendit rejected a payment session request"
        );
        return Err(AppError::ServiceUnavailable(
            "Xendit checkout could not be created. Check the sandbox account and enabled payment methods.",
        ));
    }
    response.json().await.map_err(|error| {
        tracing::warn!(%error, "Could not parse Xendit payment session response");
        AppError::ServiceUnavailable("Xendit returned an invalid checkout response.")
    })
}

async fn fetch_xendit_session(
    state: &AppState,
    session_id: &str,
) -> Result<XenditSession, AppError> {
    if !valid_session_id(session_id) {
        return Err(AppError::BadRequest(
            "Invalid Xendit payment-session ID.".into(),
        ));
    }
    let api_key = state
        .config
        .xendit_secret_key
        .as_deref()
        .ok_or(AppError::ServiceUnavailable(
            "XENDIT_SECRET_KEY is not configured.",
        ))?;
    let response = state
        .http
        .get(format!("{XENDIT_API_URL}/sessions/{session_id}"))
        .basic_auth(api_key, Some(""))
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(%error, "Could not retrieve Xendit payment session");
            AppError::ServiceUnavailable("Xendit payment status is temporarily unavailable.")
        })?;
    if !response.status().is_success() {
        tracing::warn!(status = %response.status(), "Xendit payment-session lookup failed");
        return Err(AppError::ServiceUnavailable(
            "Xendit payment status is temporarily unavailable.",
        ));
    }
    response.json().await.map_err(|error| {
        tracing::warn!(%error, "Could not parse Xendit payment-session response");
        AppError::ServiceUnavailable("Xendit returned an invalid payment status.")
    })
}

fn valid_session_id(session_id: &str) -> bool {
    session_id.len() == 27
        && session_id.starts_with("ps-")
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

pub async fn payment_status(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<PaymentStatus>, AppError> {
    if !has_role(&user, &["donor"]) {
        return Err(AppError::Forbidden);
    }
    let mut status = load_payment_status(&state, id, Some(user.id)).await?;
    if status.status == "pending" && state.config.payment_mode == PaymentMode::Xendit {
        let session_id = sqlx::query_scalar::<_, String>(
            "SELECT session_id FROM payment_attempts WHERE donation_id=$1 AND provider='xendit'",
        )
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
        let session = fetch_xendit_session(&state, &session_id).await?;
        process_xendit_session(&state, session).await?;
        status = load_payment_status(&state, id, Some(user.id)).await?;
    }
    Ok(Json(status))
}

pub async fn poll_pending_xendit_sessions(state: &AppState) -> Result<(), AppError> {
    if state.config.payment_mode != PaymentMode::Xendit {
        return Ok(());
    }

    let pending = sqlx::query_as::<_, (Uuid, String)>(
        r#"
        SELECT p.donation_id, p.session_id
        FROM payment_attempts p
        JOIN donations d ON d.id=p.donation_id
        WHERE p.provider='xendit'
          AND p.status='pending'
          AND d.status='pending'
          AND p.session_id IS NOT NULL
          AND p.updated_at <= now() - interval '30 seconds'
        ORDER BY p.updated_at
        LIMIT 25
        "#,
    )
    .fetch_all(&state.pool)
    .await?;

    for (donation_id, session_id) in pending {
        let result = async {
            let session = fetch_xendit_session(state, &session_id).await?;
            process_xendit_session(state, session).await
        }
        .await;

        if let Err(error) = result {
            tracing::warn!(
                %donation_id,
                %error,
                "Could not automatically verify pending Xendit payment"
            );
        }

        sqlx::query(
            "UPDATE payment_attempts SET updated_at=now() WHERE donation_id=$1 AND status='pending'",
        )
        .bind(donation_id)
        .execute(&state.pool)
        .await?;
    }

    Ok(())
}

pub async fn simulate_payment(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<PaymentStatus>, AppError> {
    if !has_role(&user, &["donor"]) {
        return Err(AppError::Forbidden);
    }
    if state.config.payment_mode != PaymentMode::Mock {
        return Err(AppError::NotFound);
    }
    let provider = sqlx::query_scalar::<_, String>(
        "SELECT provider FROM payment_attempts p JOIN donations d ON d.id=p.donation_id WHERE p.donation_id=$1 AND d.donor_id=$2",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    if provider != "mock" {
        return Err(AppError::NotFound);
    }
    confirm_payment(&state, id, "mock").await?;
    Ok(Json(load_payment_status(&state, id, Some(user.id)).await?))
}

async fn load_payment_status(
    state: &AppState,
    donation_id: Uuid,
    donor_id: Option<Uuid>,
) -> Result<PaymentStatus, AppError> {
    sqlx::query_as::<_, PaymentStatus>(
        r#"
        SELECT d.id, d.campaign_id, d.amount, d.currency, d.status,
               p.method AS payment_method, p.checkout_url,
               ($3::boolean) AS test_mode
        FROM donations d
        JOIN payment_attempts p ON p.donation_id=d.id
        WHERE d.id=$1 AND ($2::uuid IS NULL OR d.donor_id=$2)
        "#,
    )
    .bind(donation_id)
    .bind(donor_id)
    .bind(test_mode(state))
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)
}

async fn mark_payment_failed(state: &AppState, donation_id: Uuid) -> Result<(), AppError> {
    let mut transaction = state.pool.begin().await?;
    sqlx::query(
        "UPDATE payment_attempts SET status='failed', updated_at=now() WHERE donation_id=$1 AND status='pending'",
    )
    .bind(donation_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("UPDATE donations SET status='failed' WHERE id=$1 AND status='pending'")
        .bind(donation_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

async fn confirm_payment(
    state: &AppState,
    donation_id: Uuid,
    expected_provider: &str,
) -> Result<(), AppError> {
    let mut transaction = state.pool.begin().await?;
    let record = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            String,
            String,
            String,
            Decimal,
            Option<Decimal>,
            Option<String>,
            String,
            String,
        ),
    >(
        r#"
        SELECT d.campaign_id, d.donor_id, d.status, d.currency, c.title,
               d.amount, d.payment_amount, d.payment_currency,
               p.provider, p.method
        FROM donations d
        JOIN campaigns c ON c.id=d.campaign_id
        JOIN payment_attempts p ON p.donation_id=d.id
        WHERE d.id=$1
        FOR UPDATE OF d, p
        "#,
    )
    .bind(donation_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(AppError::NotFound)?;
    let (
        campaign_id,
        donor_id,
        donation_status,
        currency,
        campaign_title,
        amount,
        payment_amount,
        payment_currency,
        provider,
        method,
    ) = record;
    if provider != expected_provider {
        return Err(AppError::Conflict(
            "The payment provider does not match this contribution.".into(),
        ));
    }
    if donation_status == "confirmed" {
        transaction.commit().await?;
        return Ok(());
    }
    if donation_status != "pending"
        || currency != "PHP"
        || payment_amount != Some(amount)
        || payment_currency.as_deref() != Some("PHP")
    {
        return Err(AppError::Conflict(
            "This contribution cannot be confirmed.".into(),
        ));
    }
    let updated = sqlx::query(
        "UPDATE payment_attempts SET status='paid', paid_at=now(), updated_at=now() WHERE donation_id=$1 AND status='pending'",
    )
    .bind(donation_id)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if updated != 1 {
        return Err(AppError::Conflict(
            "This payment is no longer awaiting confirmation.".into(),
        ));
    }
    sqlx::query("UPDATE donations SET status='confirmed', confirmed_at=now() WHERE id=$1")
        .bind(donation_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "UPDATE campaigns SET raised_amount=raised_amount+$2, updated_at=now() WHERE id=$1",
    )
    .bind(campaign_id)
    .bind(amount)
    .execute(&mut *transaction)
    .await?;
    let summary = format!("PHP {amount} via {method}");
    activity_repository::notify_user(
        &mut transaction,
        donor_id,
        "Contribution confirmed",
        &format!("Your {summary} contribution to {campaign_title} is confirmed."),
        Some("/my-donations"),
    )
    .await?;
    let organizer_id =
        sqlx::query_scalar::<_, Uuid>("SELECT organizer_id FROM campaigns WHERE id=$1")
            .bind(campaign_id)
            .fetch_one(&mut *transaction)
            .await?;
    activity_repository::notify_user(
        &mut transaction,
        organizer_id,
        "Campaign contribution confirmed",
        &format!("A {summary} contribution to {campaign_title} is confirmed."),
        Some("/organizer/donations"),
    )
    .await?;
    activity_repository::record_audit(
        &mut transaction,
        Some(donor_id),
        "donation.confirmed",
        "donation",
        Some(donation_id),
        json!({
            "provider": provider,
            "payment_method": method,
            "campaign_amount": amount,
            "campaign_currency": currency,
            "payment_amount": payment_amount,
            "payment_currency": payment_currency
        }),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn process_xendit_session(state: &AppState, session: XenditSession) -> Result<(), AppError> {
    let reference_id = Uuid::parse_str(&session.reference_id)
        .map_err(|_| AppError::BadRequest("Invalid Xendit payment reference.".into()))?;
    let record = sqlx::query_as::<_, (Uuid, Decimal, Option<String>, String, Option<String>)>(
        r#"
        SELECT p.donation_id, d.amount, d.payment_currency, p.method, p.session_id
        FROM payment_attempts p
        JOIN donations d ON d.id=p.donation_id
        WHERE p.reference_id=$1 AND p.provider='xendit'
        "#,
    )
    .bind(session.reference_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let (donation_id, expected_amount, expected_currency, method, expected_session_id) = record;
    if donation_id != reference_id
        || expected_session_id.as_deref() != Some(session.payment_session_id.as_str())
        || Decimal::from_str(&session.amount.to_string()).ok() != Some(expected_amount)
        || expected_currency.as_deref() != Some(session.currency.as_str())
        || session.currency != "PHP"
        || !session_channels_match(session.allowed_payment_channels.as_deref(), &method)
    {
        return Err(AppError::Conflict(
            "The Xendit session details do not match this contribution.".into(),
        ));
    }
    match session.status.as_str() {
        "COMPLETED" => confirm_payment(state, donation_id, "xendit").await,
        "EXPIRED" | "CANCELED" => {
            let mut transaction = state.pool.begin().await?;
            sqlx::query(
                "UPDATE payment_attempts SET status='expired', updated_at=now() WHERE donation_id=$1 AND status='pending'",
            )
            .bind(donation_id)
            .execute(&mut *transaction)
            .await?;
            sqlx::query("UPDATE donations SET status='failed' WHERE id=$1 AND status='pending'")
                .bind(donation_id)
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
            Ok(())
        }
        "ACTIVE" => Ok(()),
        _ => Err(AppError::ServiceUnavailable(
            "Xendit returned an unsupported payment-session status.",
        )),
    }
}

fn token_matches(expected: &str, received: &str) -> bool {
    let expected = expected.as_bytes();
    let received = received.as_bytes();
    let mut difference = expected.len() ^ received.len();
    for index in 0..expected.len().max(received.len()) {
        difference |= usize::from(
            expected.get(index).copied().unwrap_or_default()
                ^ received.get(index).copied().unwrap_or_default(),
        );
    }
    difference == 0
}

pub async fn xendit_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<StatusCode, AppError> {
    if state.config.payment_mode != PaymentMode::Xendit {
        return Err(AppError::NotFound);
    }
    let expected_token =
        state
            .config
            .xendit_webhook_token
            .as_deref()
            .ok_or(AppError::ServiceUnavailable(
                "XENDIT_WEBHOOK_TOKEN is not configured.",
            ))?;
    let received_token = headers
        .get("x-callback-token")
        .and_then(|value| value.to_str().ok())
        .ok_or(AppError::Unauthorized)?;
    if !token_matches(expected_token, received_token) {
        return Err(AppError::Unauthorized);
    }
    let event = payload
        .get("event")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("Xendit event is missing.".into()))?;
    if !matches!(
        event,
        "payment_session.completed" | "payment_session.expired"
    ) {
        return Ok(StatusCode::OK);
    }
    let session_id = payload
        .get("data")
        .and_then(|data| data.get("payment_session_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("Xendit session ID is missing.".into()))?;
    let session = fetch_xendit_session(&state, session_id).await?;
    process_xendit_session(&state, session).await?;
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::{
        CreatePaymentRequest, build_return_url, normalize_payment_method, session_channels_match,
        token_matches, valid_checkout_url, valid_session_id, validate_contribution_amount,
    };
    use rust_decimal::Decimal;
    use serde_json::json;
    use std::str::FromStr;
    use uuid::Uuid;

    #[test]
    fn deserializes_gcash_request_without_donor_wallet_address() {
        let request: CreatePaymentRequest = serde_json::from_value(json!({
            "campaign_id": "00000000-0000-0000-0000-000000000001",
            "amount": 25.50,
            "payment_method": "GCASH",
            "is_anonymous": false,
            "message": ""
        }))
        .unwrap();

        assert_eq!(request.amount, Decimal::from_str("25.50").unwrap());
    }

    #[test]
    fn normalizes_supported_payment_methods() {
        assert_eq!(normalize_payment_method("gcash").unwrap(), "GCASH");
        assert_eq!(normalize_payment_method("Maya").unwrap(), "MAYA");
        assert_eq!(normalize_payment_method("PAYMAYA").unwrap(), "MAYA");
        assert!(normalize_payment_method("card").is_err());
    }

    #[test]
    fn compares_webhook_tokens_without_accepting_prefixes() {
        assert!(token_matches("secret-token", "secret-token"));
        assert!(!token_matches("secret-token", "secret"));
        assert!(!token_matches("secret-token", "secret-token-extra"));
    }

    #[test]
    fn validates_php_payment_limits_and_precision() {
        assert!(validate_contribution_amount(Decimal::ONE).is_ok());
        assert!(validate_contribution_amount(Decimal::from(100_000)).is_ok());
        assert!(validate_contribution_amount(Decimal::from_str("10.25").unwrap()).is_ok());
        assert!(validate_contribution_amount(Decimal::from_str("0.99").unwrap()).is_err());
        assert!(validate_contribution_amount(Decimal::from_str("100000.01").unwrap()).is_err());
        assert!(validate_contribution_amount(Decimal::from_str("1.001").unwrap()).is_err());
    }

    #[test]
    fn only_accepts_xendit_hosted_checkout_urls_and_session_ids() {
        assert!(valid_checkout_url(
            "https://checkout-staging.xendit.co/sessions/ps-661f87c614802d6c402cd82d"
        ));
        assert!(valid_checkout_url("https://dev.xen.to/kGxPCi76"));
        assert!(!valid_checkout_url("http://checkout.xendit.co/session"));
        assert!(!valid_checkout_url(
            "https://checkout.xendit.co.attacker.test/session"
        ));
        assert!(valid_session_id("ps-661f87c614802d6c402cd82d"));
        assert!(!valid_session_id("ps-../../admin"));
    }

    #[test]
    fn validates_returned_xendit_payment_channels() {
        let gcash = vec!["GCASH".to_owned()];
        let maya = vec!["PAYMAYA".to_owned()];
        let all_channels = vec!["GCASH".to_owned(), "PAYMAYA".to_owned()];
        assert!(session_channels_match(Some(&gcash), "GCASH"));
        assert!(session_channels_match(Some(&maya), "MAYA"));
        assert!(!session_channels_match(Some(&gcash), "MAYA"));
        assert!(!session_channels_match(Some(&all_channels), "GCASH"));
        assert!(session_channels_match(None, "MAYA"));
    }

    #[test]
    fn returns_successful_checkout_to_donor_dashboard() {
        let donation_id = Uuid::nil();
        let campaign_id = Uuid::max();

        assert_eq!(
            build_return_url("https://fundora.example", campaign_id, donation_id, false).as_deref(),
            Some(
                "https://fundora.example/dashboard?paymentDonation=00000000-0000-0000-0000-000000000000"
            )
        );
        assert_eq!(
            build_return_url("https://fundora.example", campaign_id, donation_id, true).as_deref(),
            Some(
                "https://fundora.example/campaigns/ffffffff-ffff-ffff-ffff-ffffffffffff?paymentCanceled=00000000-0000-0000-0000-000000000000"
            )
        );
    }

    #[test]
    fn omits_non_https_return_urls() {
        let donation_id = Uuid::nil();
        let campaign_id = Uuid::nil();

        assert!(
            build_return_url("http://localhost:5173", campaign_id, donation_id, false).is_none()
        );
        assert!(
            build_return_url("http://fundora.example", campaign_id, donation_id, false).is_none()
        );
    }
}
