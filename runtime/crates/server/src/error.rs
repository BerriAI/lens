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
