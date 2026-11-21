pub mod auth;
pub mod campaigns;
pub mod donations;
pub mod transactions;
pub mod withdrawals;

use axum::{
    extract::State,
    http::Request,
    middleware::{self,Next},
    response::Response,
    routing::{get,post},
    Router
};
use sqlx::Row;
use crate::{
    app::AppState,
    auth::{jwt, AuthUser},
    error::AppError
};

async fn auth_layer(State(s):State<AppState>,mut req:Request<axum::body::Body>,next:Next)->Result<Response,AppError>{
    let path = req.uri().path();
    let method = req.method();
    
    if (path.contains("/auth/") && !path.ends_with("/auth/me"))
        || (*method==axum::http::Method::GET && !path.ends_with("/auth/me"))
        || path.contains("/webhooks/blockchain/confirm/") {
        return Ok(next.run(req).await)
    }

    let token = req.headers()
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|v|v.to_str().ok())
                        .and_then(|v|v.strip_prefix("Bearer "))
                        .ok_or(AppError::Unauthorized)?;
    
    let claims = jwt::verify(token,&s.config.jwt_secret,&s.config.jwt_issuer)?;
    
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
        "#
    )
    .bind(claims_sub)
    .fetch_optional(&s.pool)
    .await?
    .ok_or(AppError::Unauthorized)?;

    let email = row.try_get::<String, _>("email")
        .map_err(|_| AppError::Unauthorized)?;
    let role = row.try_get::<Option<String>, _>("name")
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
pub fn router(s:AppState)->Router{
    Router::new()
        .route("/auth/email/request",post(auth::request_email_code)).route("/auth/email/verify",post(auth::verify_email_code))
        .route("/auth/refresh",post(auth::refresh)).route("/auth/logout",post(auth::logout)).route("/auth/me",get(auth::me))
        .route("/auth/google",post(crate::auth::providers::google)).route("/auth/apple",post(crate::auth::providers::apple))
        .route("/auth/siwe/nonce",post(crate::auth::siwe::nonce)).route("/auth/siwe/verify",post(crate::auth::siwe::verify))
        .route("/campaigns",get(campaigns::list).post(campaigns::create)).route("/campaigns/{id}",get(campaigns::get).patch(campaigns::update))
        .route("/campaigns/{id}/donations",get(donations::list_for_campaign)).route("/donations",post(donations::create))
        .route("/transactions",post(transactions::record)).route("/webhooks/blockchain/confirm/{id}",post(transactions::confirm))
        .route("/withdrawals",post(withdrawals::create)).route("/withdrawals/{id}/review",post(withdrawals::review))
        .layer(middleware::from_fn_with_state(s.clone(),auth_layer)).with_state(s)
}
