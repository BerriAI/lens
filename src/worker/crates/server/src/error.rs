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
