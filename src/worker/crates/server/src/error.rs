use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Not Found")]
    NotFound,
    #[error("Not authenticated")]
    Unauthorized,
    #[error("Invalid request: {0}")]
    InvalidRequest(&'static str),
    #[error("Internal Server Error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Authentication(#[from] lens_auth::Error),
    #[error("invalid session request")]
    Validation(Vec<serde_json::Value>),
    #[error("session could not be generated")]
    Entropy(#[source] rand::Error),
}

impl IntoResponse for SessionError {
    fn into_response(self) -> Response {
        use lens_auth::{Error, StoreError};
        let (status, detail) = match self {
            Self::Validation(errors) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                serde_json::Value::Array(errors),
            ),
            Self::Entropy(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                serde_json::Value::from("Lens is temporarily unavailable"),
            ),
            Self::Authentication(error) => {
                let status = match &error {
                    Error::Unauthorized(_) => StatusCode::UNAUTHORIZED,
                    Error::OriginMismatch => StatusCode::FORBIDDEN,
                    Error::Store(StoreError::Conflict) => StatusCode::CONFLICT,
                    Error::Store(StoreError::Unavailable(_)) | Error::Configuration(_) => {
                        StatusCode::SERVICE_UNAVAILABLE
                    }
                };
                (status, serde_json::Value::from(error.to_string()))
            }
        };
        (status, Json(serde_json::json!({"detail": detail}))).into_response()
    }
}

#[derive(Serialize)]
struct ErrorBody {
    detail: String,
    code: &'static str,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::InvalidRequest(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"),
            Self::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };
        (
            status,
            Json(ErrorBody {
                detail: self.to_string(),
                code,
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::to_bytes, http::StatusCode, response::IntoResponse};
    use rstest::rstest;

    use super::ApiError;

    #[rstest]
    #[case::not_found(
        ApiError::NotFound,
        StatusCode::NOT_FOUND,
        r#"{"detail":"Not Found","code":"not_found"}"#
    )]
    #[case::unauthorized(
        ApiError::Unauthorized,
        StatusCode::UNAUTHORIZED,
        r#"{"detail":"Not authenticated","code":"unauthorized"}"#
    )]
    #[case::internal(
        ApiError::Internal(Box::new(std::io::Error::other("private source detail"))),
        StatusCode::INTERNAL_SERVER_ERROR,
        r#"{"detail":"Internal Server Error","code":"internal_error"}"#
    )]
    #[tokio::test]
    async fn api_errors_return_generic_json(
        #[case] error: ApiError,
        #[case] expected_status: StatusCode,
        #[case] expected_body: &str,
    ) {
        let response = error.into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("error response body is readable");

        assert_eq!(status, expected_status);
        assert_eq!(body.as_ref(), expected_body.as_bytes());
    }
}

#[cfg(test)]
mod session_tests {
    use super::SessionError;
    use axum::{body::to_bytes, response::IntoResponse};
    use lens_auth::{Error, StoreError};
    use rstest::rstest;
    use serde_json::{Value, json};

    #[rstest]
    #[case::unauthorized(Error::Unauthorized("Sign in to Lens"), 401, "Sign in to Lens")]
    #[case::origin(
        Error::OriginMismatch,
        403,
        "Lens session requests must come from the Lens origin"
    )]
    #[case::conflict(
        Error::Store(StoreError::Conflict),
        409,
        "Lens state changed; retry the operation"
    )]
    #[case::storage(
        Error::Store(StoreError::Unavailable(Box::new(std::io::Error::other(
            "private database detail"
        )))),
        503,
        "Lens storage is unavailable"
    )]
    #[tokio::test]
    async fn authentication_errors_preserve_python_shape(
        #[case] error: Error,
        #[case] status: u16,
        #[case] message: &str,
    ) {
        let response = SessionError::from(error).into_response();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["content-type"], "application/json");
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"detail":message})
        );
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvalApiError {
    #[error("Unsupported Lens contract version; send X-Lens-Contract: 1")]
    ContractVersion,
    #[error("Dataset not found")]
    DatasetNotFound,
    #[error("Dataset revision not found")]
    RevisionNotFound,
    #[error("Case does not belong to this run")]
    UnknownCase,
    #[error("Not authenticated for an eval team")]
    Unauthorized,
    #[error(transparent)]
    Authentication(#[from] lens_auth::Error),
    #[error(transparent)]
    EvalStore(#[from] litellm_storage_clickhouse::EvalError),
    #[error(transparent)]
    Storage(#[from] litellm_storage_clickhouse::Error),
    #[error(transparent)]
    Request(#[from] ApiError),
    #[error("Invalid dataset state")]
    Dataset(#[from] serde_json::Error),
}

impl IntoResponse for EvalApiError {
    fn into_response(self) -> Response {
        use lens_contract::eval::{ApiError as Body, ApiErrorCode};
        use litellm_storage_clickhouse::EvalError;
        let detail = self.to_string();
        let (status, code) = match self {
            Self::ContractVersion => (StatusCode::CONFLICT, ApiErrorCode::ContractVersion),
            Self::DatasetNotFound => (StatusCode::NOT_FOUND, ApiErrorCode::DatasetNotFound),
            Self::RevisionNotFound => (StatusCode::NOT_FOUND, ApiErrorCode::RevisionNotFound),
            Self::UnknownCase | Self::EvalStore(EvalError::UnknownCase) => {
                (StatusCode::NOT_FOUND, ApiErrorCode::UnknownCase)
            }
            Self::Unauthorized | Self::Authentication(lens_auth::Error::Unauthorized(_)) => {
                (StatusCode::UNAUTHORIZED, ApiErrorCode::Unauthorized)
            }
            Self::Authentication(lens_auth::Error::OriginMismatch) => {
                (StatusCode::FORBIDDEN, ApiErrorCode::Unauthorized)
            }
            Self::EvalStore(EvalError::RunNotFound) => {
                (StatusCode::NOT_FOUND, ApiErrorCode::RunNotFound)
            }
            Self::EvalStore(EvalError::RunClosed) => {
                (StatusCode::CONFLICT, ApiErrorCode::RunClosed)
            }
            Self::EvalStore(EvalError::InvalidTrial) => {
                return ApiError::InvalidRequest("trial is outside the run's trial range")
                    .into_response();
            }
            Self::EvalStore(EvalError::InvalidRequest(message)) => {
                return ApiError::InvalidRequest(message).into_response();
            }
            Self::EvalStore(EvalError::IdempotencyConflict) => {
                return ApiError::InvalidRequest("idempotency key belongs to a different request")
                    .into_response();
            }
            Self::Request(error) => return error.into_response(),
            error => return ApiError::Internal(Box::new(error)).into_response(),
        };
        (status, Json(Body { detail, code })).into_response()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvalCloserError {
    #[error(transparent)]
    Storage(#[from] litellm_storage_clickhouse::EvalError),
    #[error("Eval trace lookup failed")]
    Traces(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("Eval scoring failed")]
    Scoring(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("Eval scoring is not configured on this Lens instance")]
    ScoringUnavailable,
    #[error("Stored eval run is incomplete")]
    InvalidRun,
}
