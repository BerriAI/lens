use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum TraceReadError {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("Trace changed while paging; refresh the trace to continue")]
    Changed,
    #[error("Trace is too large for this view. Use a filtered trace query.")]
    TooLarge,
    #[error("Traces are temporarily unavailable. Please try again.")]
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TraceHttpError {
    #[error(transparent)]
    Authentication(#[from] lens_auth::Error),
    #[error(transparent)]
    Validation(#[from] DatasetError),
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("Trace {0} not found")]
    MissingTrace(String),
    #[error("Span {0} not found")]
    MissingSpan(String),
    #[error("Span diagnostic not found or no longer available")]
    MissingDiagnostic,
    #[error("{source}")]
    Read {
        source: TraceReadError,
        retry_after_seconds: i64,
    },
    #[error("{0}")]
    Query(TraceReadError),
    #[error("Trace query help is temporarily unavailable")]
    Help,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum IngestionHttpError {
    #[error(transparent)]
    Authentication(#[from] lens_auth::Error),
    #[error(transparent)]
    Validation(#[from] DatasetError),
    #[error(transparent)]
    Ingestion(#[from] lens_auth::IngestionError),
}

impl IntoResponse for IngestionHttpError {
    fn into_response(self) -> Response {
        use lens_auth::IngestionError;
        let error = match self {
            Self::Authentication(error) => return SessionError::from(error).into_response(),
            Self::Validation(error) => return error.into_response(),
            Self::Ingestion(error) => error,
        };
        let status = match &error {
            IngestionError::Forbidden(_) => StatusCode::FORBIDDEN,
            IngestionError::InvalidExpiry | IngestionError::InvalidLength { .. } => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            IngestionError::AlreadyExists | IngestionError::KeyLimit => StatusCode::CONFLICT,
            IngestionError::Store(lens_auth::StoreError::Conflict) => StatusCode::CONFLICT,
            IngestionError::CatalogLimit | IngestionError::Random(_) | IngestionError::Store(_) => {
                StatusCode::SERVICE_UNAVAILABLE
            }
        };
        (
            status,
            Json(serde_json::json!({"detail":error.to_string()})),
        )
            .into_response()
    }
}

impl IntoResponse for TraceHttpError {
    fn into_response(self) -> Response {
        use serde_json::json;
        let (status, detail) = match self {
            Self::Authentication(error) => return SessionError::from(error).into_response(),
            Self::Validation(error) => return error.into_response(),
            error @ Self::Forbidden(_) => (StatusCode::FORBIDDEN, error.to_string()),
            error @ (Self::MissingTrace(_) | Self::MissingSpan(_) | Self::MissingDiagnostic) => {
                (StatusCode::NOT_FOUND, error.to_string())
            }
            Self::Read {
                source,
                retry_after_seconds,
            } => {
                let (status, code) = match source {
                    TraceReadError::InvalidRequest(_) => {
                        (StatusCode::BAD_REQUEST, "invalid_request")
                    }
                    TraceReadError::Changed => (StatusCode::CONFLICT, "trace_changed"),
                    TraceReadError::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "too_large"),
                    TraceReadError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
                };
                let mut response = (
                    status,
                    Json(json!({"detail":{"code":code,"message":source.to_string()}})),
                )
                    .into_response();
                if matches!(source, TraceReadError::Unavailable)
                    && let Ok(header) = retry_after_seconds.to_string().parse()
                {
                    response
                        .headers_mut()
                        .insert(axum::http::header::RETRY_AFTER, header);
                }
                return response;
            }
            Self::Query(TraceReadError::InvalidRequest(message)) => {
                (StatusCode::BAD_REQUEST, message)
            }
            Self::Query(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Trace SQL query failed or exceeded reader limits".to_owned(),
            ),
            Self::Help => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
        };
        (status, Json(json!({"detail":detail}))).into_response()
    }
}

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

#[derive(Debug, thiserror::Error)]
pub enum InvestigationAccessError {
    #[error("Choose a model configured on this LiteLLM instance")]
    ModelUnavailable,
    #[error("This key does not have access to the analysis model")]
    ModelForbidden,
    #[error(
        "No worker can use this analysis model. Choose a model available to the worker's virtual key, or update its model access and pricing."
    )]
    WorkerUnavailable,
    #[error("{reason}")]
    Rejected { status: StatusCode, reason: String },
    #[error("Lens is temporarily unavailable")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl InvestigationAccessError {
    fn status(&self) -> StatusCode {
        match self {
            Self::ModelUnavailable | Self::WorkerUnavailable => StatusCode::BAD_REQUEST,
            Self::ModelForbidden => StatusCode::FORBIDDEN,
            Self::Rejected { status, .. } => *status,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InvestigationError {
    #[error(transparent)]
    Authentication(#[from] lens_auth::Error),
    #[error(transparent)]
    Storage(#[from] lens_investigations::RepositoryError),
    #[error(transparent)]
    Request(#[from] DatasetError),
    #[error(transparent)]
    Access(#[from] InvestigationAccessError),
    #[error(transparent)]
    Domain(#[from] lens_investigations::Error),
    #[error("Lens requires proxy administrator access")]
    ForbiddenRead,
    #[error("Only proxy admins can configure or run Lens")]
    ForbiddenWrite,
    #[error("Lens not found")]
    NotFound,
    #[error("Investigation not found")]
    RunNotFound,
    #[error("Lens changed concurrently; retry the operation")]
    Changed,
    #[error("Choose execution IDs returned by the activity preview")]
    InvalidSelection,
}

impl IntoResponse for InvestigationError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Authentication(_) | Self::Request(_) => {
                return match self {
                    Self::Authentication(error) => SessionError::from(error).into_response(),
                    Self::Request(error) => error.into_response(),
                    _ => unreachable!(),
                };
            }
            Self::Storage(lens_investigations::RepositoryError::Conflict) | Self::Changed => {
                StatusCode::CONFLICT
            }
            Self::Storage(lens_investigations::RepositoryError::Unavailable(_)) => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            Self::Access(error) => error.status(),
            Self::Domain(_) | Self::InvalidSelection => StatusCode::UNPROCESSABLE_ENTITY,
            Self::ForbiddenRead | Self::ForbiddenWrite => StatusCode::FORBIDDEN,
            Self::NotFound | Self::RunNotFound => StatusCode::NOT_FOUND,
        };
        (status, Json(serde_json::json!({"detail":self.to_string()}))).into_response()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Lens feedback is temporarily unavailable")]
pub struct FeedbackStoreError(#[source] pub Box<dyn std::error::Error + Send + Sync>);

#[derive(Debug, thiserror::Error)]
pub enum FeedbackError {
    #[error(transparent)]
    Authentication(#[from] lens_auth::Error),
    #[error(transparent)]
    Request(#[from] DatasetError),
    #[error(transparent)]
    Storage(#[from] FeedbackStoreError),
    #[error("Lens requires proxy administrator access")]
    ForbiddenRead,
    #[error("Admin viewers cannot write feedback")]
    ForbiddenViewer,
    #[error("Feedback requires a team or API key")]
    ScopeRequired,
    #[error("Name the user who left this feedback")]
    AuthorRequired,
    #[error("Trace not found")]
    TraceNotFound,
    #[error("No feedback from this user on this trace")]
    FeedbackNotFound,
}

impl IntoResponse for FeedbackError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Authentication(_) | Self::Request(_) => {
                return match self {
                    Self::Authentication(error) => SessionError::from(error).into_response(),
                    Self::Request(error) => error.into_response(),
                    _ => unreachable!(),
                };
            }
            Self::Storage(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::ForbiddenRead | Self::ForbiddenViewer | Self::ScopeRequired => {
                StatusCode::FORBIDDEN
            }
            Self::AuthorRequired => StatusCode::UNPROCESSABLE_ENTITY,
            Self::TraceNotFound | Self::FeedbackNotFound => StatusCode::NOT_FOUND,
        };
        (status, Json(serde_json::json!({"detail":self.to_string()}))).into_response()
    }
}
#[derive(Debug, thiserror::Error)]
#[error("Lens activity is temporarily unavailable")]
pub struct ActivityReadError(#[source] pub Box<dyn std::error::Error + Send + Sync>);

#[derive(Debug, thiserror::Error)]
pub enum ActivityError {
    #[error(transparent)]
    Authentication(#[from] lens_auth::Error),
    #[error(transparent)]
    Request(#[from] DatasetError),
    #[error(transparent)]
    Investigation(#[from] InvestigationError),
    #[error(transparent)]
    Storage(#[from] ActivityReadError),
    #[error("Preview window exceeds the supported calendar range")]
    Window,
    #[error("Execution not found")]
    ExecutionNotFound,
    #[error("Agent tracing is not enabled. Configure the Lens service and LITELLM_LENS_URL.")]
    TracingDisabled,
    #[error("Internal Server Error")]
    Transport,
}

impl IntoResponse for ActivityError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Authentication(_) | Self::Request(_) | Self::Investigation(_) => {
                return match self {
                    Self::Authentication(error) => SessionError::from(error).into_response(),
                    Self::Request(error) => error.into_response(),
                    Self::Investigation(error) => error.into_response(),
                    _ => unreachable!(),
                };
            }
            Self::Transport => {
                return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
                    .into_response();
            }
            Self::Storage(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Window => StatusCode::UNPROCESSABLE_ENTITY,
            Self::ExecutionNotFound => StatusCode::NOT_FOUND,
            Self::TracingDisabled => StatusCode::NOT_IMPLEMENTED,
        };
        (status, Json(serde_json::json!({"detail":self.to_string()}))).into_response()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error(transparent)]
    Authentication(#[from] lens_auth::Error),
    #[error(transparent)]
    Dataset(#[from] lens_datasets::StoreError),
    #[error(transparent)]
    Run(#[from] lens_evals::RunError),
    #[error("unauthorized")]
    Forbidden,
    #[error("contract_version")]
    Contract,
    #[error("idempotency_key")]
    Idempotency,
    #[error("dataset_not_found")]
    DatasetNotFound,
    #[error("revision_not_found")]
    RevisionNotFound,
    #[error("invalid_request")]
    InvalidRequest(#[source] serde_json::Error),
    #[error("invalid_result")]
    InvalidResult(#[source] serde_json::Error),
    #[error("invalid_request")]
    InvalidSpec,
}

impl IntoResponse for EvalError {
    fn into_response(self) -> Response {
        use lens_evals::RunError;
        let (status, code) = match &self {
            Self::Authentication(_) => {
                return match self {
                    Self::Authentication(error) => SessionError::from(error).into_response(),
                    _ => unreachable!(),
                };
            }
            Self::Forbidden => (StatusCode::FORBIDDEN, "unauthorized"),
            Self::Contract => (StatusCode::CONFLICT, "contract_version"),
            Self::Idempotency => (StatusCode::UNPROCESSABLE_ENTITY, "idempotency_key"),
            Self::DatasetNotFound => (StatusCode::NOT_FOUND, "dataset_not_found"),
            Self::RevisionNotFound => (StatusCode::NOT_FOUND, "revision_not_found"),
            Self::InvalidSpec | Self::InvalidRequest(_) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request")
            }
            Self::InvalidResult(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_result"),
            Self::Dataset(_) => (StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable"),
            Self::Run(error) => match error {
                RunError::NotFound => (StatusCode::NOT_FOUND, "run_not_found"),
                RunError::UnknownCase => (StatusCode::UNPROCESSABLE_ENTITY, "unknown_case"),
                RunError::InvalidTrial | RunError::InvalidResult => {
                    (StatusCode::UNPROCESSABLE_ENTITY, "invalid_result")
                }
                RunError::Closed => (StatusCode::CONFLICT, "run_closed"),
                RunError::IdempotencyConflict => (StatusCode::CONFLICT, "idempotency_key"),
                RunError::InvalidRun => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"),
                RunError::StaleLease | RunError::Conflict => {
                    (StatusCode::CONFLICT, "state_conflict")
                }
                RunError::Unavailable(_) => {
                    (StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable")
                }
            },
        };
        (status, Json(serde_json::json!({"detail":code,"code":code}))).into_response()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SignalError {
    #[error(transparent)]
    Authentication(#[from] lens_auth::Error),
    #[error(transparent)]
    Request(#[from] DatasetError),
    #[error(transparent)]
    Investigation(#[from] InvestigationError),
    #[error(transparent)]
    Storage(#[from] lens_signals::RepositoryError),
    #[error("Lens signal storage is unavailable")]
    Invalid(#[from] lens_signals::Error),
    #[error("Choose a System 1 model (evaluation mode) configured on this proxy")]
    Model,
}

impl IntoResponse for SignalError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Authentication(_) | Self::Request(_) | Self::Investigation(_) => {
                return match self {
                    Self::Authentication(error) => SessionError::from(error).into_response(),
                    Self::Request(error) => error.into_response(),
                    Self::Investigation(error) => error.into_response(),
                    _ => unreachable!(),
                };
            }
            Self::Storage(_) | Self::Invalid(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Model => StatusCode::BAD_REQUEST,
        };
        (status, Json(serde_json::json!({"detail":self.to_string()}))).into_response()
    }
}
