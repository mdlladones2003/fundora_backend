pub mod admin;
pub mod auth;
pub mod campaigns;
pub mod dashboard;
pub mod donations;
pub mod notifications;
pub mod payments;
pub mod role_requests;
pub mod transactions;
pub mod withdrawals;

use crate::{
    app::AppState,
    auth::{AuthUser, jwt},
    error::AppError,
};
use axum::{
    Router,
    extract::State,
    http::Request,
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use sqlx::Row;

async fn auth_layer(
    State(s): State<AppState>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    let path = req.uri().path();
    let method = req.method();

    let public_auth = [
        "/auth/email/request",
        "/auth/email/verify",
        "/auth/refresh",
        "/auth/logout",
        "/auth/google",
        "/auth/apple",
        "/auth/siwe/nonce",
        "/auth/siwe/verify",
    ]
    .iter()
    .any(|suffix| path.ends_with(suffix));
    let public_campaign_read = *method == axum::http::Method::GET
        && (path.ends_with("/campaigns") || path.contains("/campaigns/"))
        && !path.ends_with("/campaigns/mine")
        && !path.contains("/organizer/")
        && !path.contains("/admin/");
    let public_campaign_view = *method == axum::http::Method::POST
        && path.contains("/campaigns/")
        && path.ends_with("/views");
    if public_auth
        || public_campaign_read
        || public_campaign_view
        || path.ends_with("/payments/config")
        || path.ends_with("/webhooks/xendit")
    {
        return Ok(next.run(req).await);
    }

    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(AppError::Unauthorized)?;

    let claims = jwt::verify(token, &s.config.jwt_secret, &s.config.jwt_issuer)?;

    let claims_sub = claims.sub;
    let row = sqlx::query(
        r#"
        SELECT u.email, r.name
        FROM users u
        LEFT JOIN user_roles ur ON ur.user_id = u.id
        LEFT JOIN roles r ON r.id = ur.role_id
        WHERE u.id = $1 AND u.is_active = TRUE
        ORDER BY r.name = 'administrator' DESC,
                 r.name = 'organizer' DESC,
                 r.name = 'donor' DESC
        LIMIT 1
        "#,
    )
    .bind(claims_sub)
    .fetch_optional(&s.pool)
    .await?
    .ok_or(AppError::Unauthorized)?;

    let email = row
        .try_get::<String, _>("email")
        .map_err(|_| AppError::Unauthorized)?;
    let role = row
        .try_get::<Option<String>, _>("name")
        .ok()
        .flatten()
        .unwrap_or_else(|| "guest".into());

    req.extensions_mut().insert(claims);
    req.extensions_mut().insert(AuthUser {
        id: claims_sub,
        email,
        role,
    });
    Ok(next.run(req).await)
}
pub fn router(s: AppState) -> Router {
    Router::new()
        .route("/auth/email/request", post(auth::request_email_code))
        .route("/auth/email/verify", post(auth::verify_email_code))
        .route("/auth/refresh", post(auth::refresh))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me).patch(auth::update_me))
        .route("/auth/google", post(crate::auth::providers::google))
        .route("/auth/apple", post(crate::auth::providers::apple))
        .route("/auth/siwe/nonce", post(crate::auth::siwe::nonce))
        .route("/auth/siwe/verify", post(crate::auth::siwe::verify))
        .route(
            "/auth/wallet/link/nonce",
            post(crate::auth::siwe::wallet_link_nonce),
        )
        .route("/auth/wallet/link", post(crate::auth::siwe::link_wallet))
        .route("/campaigns", get(campaigns::list).post(campaigns::create))
        .route("/campaigns/mine", get(campaigns::list_mine))
        .route(
            "/organizer/campaigns/analytics",
            get(campaigns::organizer_analytics),
        )
        .route(
            "/campaigns/{id}",
            get(campaigns::get).patch(campaigns::update),
        )
        .route("/campaigns/{id}/views", post(campaigns::track_view))
        .route("/donations", post(payments::create))
        .route("/donations/my", get(donations::list_mine))
        .route("/donations/{id}", get(donations::get_mine))
        .route("/donations/{id}/verify", post(donations::verify))
        .route(
            "/donations/{id}/payment-status",
            get(payments::payment_status),
        )
        .route(
            "/donations/{id}/simulate-payment",
            post(payments::simulate_payment),
        )
        .route("/webhooks/xendit", post(payments::xendit_webhook))
        .route("/payments/config", get(donations::payment_config))
        .route("/notifications/mine", get(notifications::list_mine))
        .route(
            "/notifications/{id}/read",
            axum::routing::patch(notifications::mark_read),
        )
        .route("/dashboard/summary", get(dashboard::summary))
        .route("/organizer/donations", get(dashboard::organizer_donations))
        .route(
            "/role-requests/organizer",
            get(role_requests::get_mine).post(role_requests::request_organizer),
        )
        .route("/admin/role-requests", get(role_requests::list_pending))
        .route(
            "/admin/role-requests/{id}/review",
            post(role_requests::review),
        )
        .route("/admin/summary", get(admin::summary))
        .route("/admin/campaigns", get(admin::list_campaigns))
        .route(
            "/admin/campaigns/{id}",
            axum::routing::patch(admin::update_campaign),
        )
        .route("/admin/campaigns/{id}/review", post(admin::review_campaign))
        .route("/admin/users", get(admin::list_users))
        .route(
            "/admin/users/{id}",
            axum::routing::patch(admin::update_user),
        )
        .route("/admin/donations", get(admin::list_donations))
        .route(
            "/admin/donations/{id}",
            axum::routing::patch(update_donation),
        )
        .route("/admin/audit-logs", get(admin::list_audit_logs))
        .route("/admin/transactions", get(admin::list_transactions))
        .route(
            "/admin/transactions/{id}/flag",
            axum::routing::patch(admin::flag_transaction),
        )
        .route("/admin/withdrawals", get(admin::list_withdrawals))
        .route("/transactions", post(transactions::record))
        .route("/withdrawals", post(withdrawals::create))
        .route("/withdrawals/mine", get(withdrawals::list_mine))
        .route("/withdrawals/{id}/review", post(withdrawals::review))
        .layer(middleware::from_fn_with_state(s.clone(), auth_layer))
        .with_state(s)
}

async fn update_donation() -> axum::http::StatusCode {
    axum::http::StatusCode::NOT_IMPLEMENTED
}
