use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("OIDC verification failed: {0}")]
    OidcVerify(String),

    #[error("session not found or expired")]
    SessionNotFound,

    #[error("unauthorized")]
    Unauthorized,

    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            AppError::Db(e) => {
                tracing::error!(error = %e, "database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "DB_ERROR",
                    "Internal error".to_string(),
                )
            }
            AppError::OidcVerify(msg) => {
                (StatusCode::UNAUTHORIZED, "OIDC_VERIFY_FAILED", msg.clone())
            }
            AppError::SessionNotFound => (
                StatusCode::UNAUTHORIZED,
                "SESSION_NOT_FOUND",
                "Session not found or expired".to_string(),
            ),
            AppError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "UNAUTHORIZED",
                "Unauthorized".to_string(),
            ),
            AppError::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_ERROR",
                    "Internal error".to_string(),
                )
            }
        };

        let body = Json(json!({ "error": code, "message": message }));
        (status, body).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
