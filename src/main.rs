mod app;
mod auth;
mod config;
mod database;
mod error;
mod routes;

use axum::{
    Router,
    http::{HeaderValue, Method},
    routing::get,
};
use std::{net::SocketAddr, time::Duration};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let config = config::Config::from_env()?;
    let pool = database::connection::connect(&config.database_url).await?;

    let mut schema_transaction = pool.begin().await?;
    sqlx::raw_sql(include_str!("../schema.sql"))
        .execute(&mut *schema_transaction)
        .await?;
    schema_transaction.commit().await?;

    let port = config.port;
    let mut origins: Vec<HeaderValue> = config
        .frontend_origin
        .split(',')
        .filter_map(|origin| origin.trim().parse::<HeaderValue>().ok())
        .collect();
    if let Ok(origin) = "http://localhost:3000".parse::<HeaderValue>() {
        origins.push(origin);
    }
    if let Ok(origin) = "http://127.0.0.1:3000".parse::<HeaderValue>() {
        origins.push(origin);
    }
    let state = app::AppState {
        pool,
        config,
        http: reqwest::Client::new(),
    };
    if state.config.payment_mode == config::PaymentMode::Xendit {
        let payment_state = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                if let Err(error) =
                    routes::payments::poll_pending_xendit_sessions(&payment_state).await
                {
                    tracing::error!(%error, "Xendit payment verification poll failed");
                }
            }
        });
    }
    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
        ]);
    let app = Router::new()
        .route(
            "/health",
            get(|| async { axum::Json(serde_json::json!({"status":"ok","service":"fundora"})) }),
        )
        .nest("/api/v1", routes::router(state))
        .layer(TraceLayer::new_for_http())
        .layer(cors);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!("Fundora API listening on {}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}
