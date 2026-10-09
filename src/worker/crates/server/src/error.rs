use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug)]
pub struct ValidationError {
    pub(crate) value: serde_json::Value,
    pub(crate) input: Option<Box<serde_json::value::RawValue>>,
}

impl From<serde_json::Value> for ValidationError {
    fn from(value: serde_json::Value) -> Self {
        Self { value, input: None }
    }
}

impl Serialize for ValidationError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{Error, SerializeMap};
        let fields = self
            .value
            .as_object()
            .ok_or_else(|| S::Error::custom("validation error must be an object"))?;
        let mut map = serializer.serialize_map(Some(fields.len()))?;
        for (key, value) in fields {
            if key == "input"
                && let Some(input) = &self.input
            {
                map.serialize_entry(key, input)?;
            } else {
                map.serialize_entry(key, value)?;
            }
        }
        map.end()
    }
}

#[cfg(test)]
impl std::ops::Deref for ValidationError {
    type Target = serde_json::Value;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

#[cfg(test)]
impl PartialEq<serde_json::Value> for ValidationError {
    fn eq(&self, other: &serde_json::Value) -> bool {
        &self.value == other
    }
}

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

#[derive(Debug, thiserror::Error)]
pub enum DatasetError {
    #[error(transparent)]
    Authentication(#[from] lens_auth::Error),
    #[error(transparent)]
    Storage(#[from] lens_datasets::StoreError),
    #[error("invalid dataset request")]
    Validation(Vec<ValidationError>),
    #[error("Lens requires proxy administrator access")]
    ForbiddenRead,
    #[error("Only proxy admins can configure or run Lens")]
    ForbiddenWrite,
    #[error("Dataset not found")]
    NotFound,
    #[error("Dataset already exists")]
    AlreadyExists,
    #[error("Dataset changed, reload")]
    Changed,
    #[error(transparent)]
    Revision(#[from] lens_datasets::RevisionProblem),
    #[error("{source}")]
    Read {
        source: lens_datasets::ReadError,
        retry_after_seconds: i64,
    },
    #[error("Lens is temporarily unavailable")]
    Decode(#[source] serde_json::Error),
}

impl IntoResponse for DatasetError {
    fn into_response(self) -> Response {
        use lens_datasets::{ReadError, StoreError};
        let (status, detail) = match self {
            Self::Authentication(error) => return SessionError::from(error).into_response(),
            Self::Storage(error) => (
                match error {
                    StoreError::Conflict => StatusCode::CONFLICT,
                    StoreError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
                },
                serde_json::json!(error.to_string()),
            ),
            Self::Validation(errors) => {
                #[derive(Serialize)]
                struct ValidationBody {
                    detail: Vec<ValidationError>,
                }
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(ValidationBody { detail: errors }),
                )
                    .into_response();
            }
            Self::Read {
                source: error,
                retry_after_seconds,
            } => {
                let (status, code) = match error {
                    ReadError::LensNotFound => {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({"detail": error.to_string()})),
                        )
                            .into_response();
                    }
                    ReadError::TraceChanged(_) => (StatusCode::CONFLICT, "trace_changed"),
                    ReadError::InvalidRequest(_) => (StatusCode::BAD_REQUEST, "invalid_request"),
                    ReadError::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "too_large"),
                    ReadError::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
                    ReadError::Storage(error) => return Self::Storage(error).into_response(),
                };
                let mut response = (
                    status,
                    Json(
                        serde_json::json!({"detail": {"code": code, "message": error.to_string()}}),
                    ),
                )
                    .into_response();
                if matches!(error, ReadError::Unavailable(_))
                    && let Ok(value) = retry_after_seconds.to_string().parse()
                {
                    response
                        .headers_mut()
                        .insert(axum::http::header::RETRY_AFTER, value);
                }
                return response;
            }
            error @ (Self::ForbiddenRead | Self::ForbiddenWrite) => {
                (StatusCode::FORBIDDEN, serde_json::json!(error.to_string()))
            }
            error @ Self::NotFound => (StatusCode::NOT_FOUND, serde_json::json!(error.to_string())),
            error @ (Self::AlreadyExists | Self::Changed) => {
                (StatusCode::CONFLICT, serde_json::json!(error.to_string()))
            }
            error @ Self::Revision(_) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                serde_json::json!(error.to_string()),
            ),
            error @ Self::Decode(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                serde_json::json!(error.to_string()),
            ),
        };
        (status, Json(serde_json::json!({"detail": detail}))).into_response()
    }
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
