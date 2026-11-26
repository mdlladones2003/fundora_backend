use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error")]
    Db(#[from] sqlx::Error),
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("not found")]
    NotFound,
    #[error("invalid request: {0}")]
    BadRequest(String),
    #[error("{0}")]
    ServiceUnavailable(&'static str),
    #[error("external payment provider rejected the request")]
    ProviderRejected { code: String },
    #[error("internal server error")]
    Internal,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Db(sqlx::Error::RowNotFound) | Self::NotFound => {
                (StatusCode::NOT_FOUND, "resource not found".to_string())
            }
            Self::Db(err) => {
                tracing::error!(error = %err, "Database operation failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".to_string(),
                )
            }
            Self::ServiceUnavailable(message) => {
                (StatusCode::SERVICE_UNAVAILABLE, message.to_string())
            }
            Self::ProviderRejected { code } => (
                StatusCode::SERVICE_UNAVAILABLE,
                format!(
                    "Xendit rejected checkout creation (provider code: {code}). Verify the test-mode account and selected payment method."
                ),
            ),
            Self::Conflict(message) => (StatusCode::CONFLICT, message),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".to_string(),
            ),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".into()),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden".into()),
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
        };
        (status, Json(json!({"error":message}))).into_response()
    }
}
