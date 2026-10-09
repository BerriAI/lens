#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Identity(#[from] lens_inference::Error),
    #[error("Evaluation deployment names and models must not be empty")]
    MissingModel,
    #[error("Evaluation deployment {model} requires a provider API key")]
    MissingCredential { model: String },
    #[error("Evaluation model {model} is not configured")]
    UnknownModel { model: String },
    #[error("Evaluation model {model} does not support Decisions")]
    ModelSurface { model: String },
    #[error(
        "Evaluation deployment requires a valid HTTP or HTTPS API base without credentials, query or fragment"
    )]
    Endpoint,
    #[error("Evaluation transport timeout and response limit must be greater than zero")]
    Limits,
    #[error("Evaluation provider response exceeds the configured limit")]
    ResponseLimit,
    #[error("Evaluation model request timed out")]
    Timeout,
    #[error("Evaluation provider returned HTTP {status}")]
    Provider { status: u16 },
    #[error("Evaluation provider transport failed")]
    Transport(#[source] reqwest::Error),
    #[error("Evaluation HTTP client configuration failed")]
    Http(#[from] litellm_http::Error),
    #[error("Evaluation provider returned invalid JSON")]
    Json(#[from] serde_json::Error),
}
