use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("This credential cannot access this workspace")]
    Forbidden,
    #[error("Record not found")]
    NotFound,
    #[error("{0}")]
    Conflict(&'static str),
    #[error("Agent storage is unavailable")]
    Storage(#[from] sqlx::Error),
    #[error("Agent storage could not be initialized")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("Stored agent data is invalid")]
    Json(#[from] serde_json::Error),
    #[error("Authentication failed")]
    Authentication(#[from] lens_auth::Error),
    #[error("Agent workspace is unavailable")]
    Io(#[from] std::io::Error),
    #[error("Agent runtime is unavailable")]
    Runtime,
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Authentication(_) => StatusCode::UNAUTHORIZED,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        };
        (
            status,
            Json(serde_json::json!({"detail": self.to_string()})),
        )
            .into_response()
    }
}
