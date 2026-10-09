#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Analysis deployment {model} requires a provider API key")]
    MissingCredential { model: String },
    #[error("Analysis deployment names and models must not be empty")]
    MissingModel,
    #[error("Analysis model {model} does not support chat completions")]
    ModelSurface { model: String },
    #[error("Analysis model {model} is not configured")]
    UnknownModel { model: String },
    #[error(
        "Analysis deployment requires a valid HTTP or HTTPS API base without credentials, query or fragment"
    )]
    Endpoint,
    #[error("Analysis transport timeout and response limit must be greater than zero")]
    TransportLimits,
    #[error("Analysis provider response exceeds the configured limit")]
    ResponseLimit,
    #[error("Analysis request timed out waiting for model output")]
    Timeout,
    #[error("Analysis provider returned HTTP {status}")]
    Provider {
        status: u16,
        retry_after: Option<u64>,
    },
    #[error("Analysis provider response is missing a completion")]
    MissingCompletion,
    #[error("Analysis provider returned unsupported content")]
    ResponseContent,
    #[error("Analysis conversation must open on a user turn")]
    Conversation,
    #[error("Analysis token count exceeds the supported range")]
    TokenCount,
    #[error("Analysis tokenizer task failed")]
    TokenizerTask(#[source] tokio::task::JoinError),
    #[error("Analysis provider transport failed")]
    Transport(#[source] reqwest::Error),
    #[error(transparent)]
    Policy(#[from] lens_inference::Error),
    #[error(transparent)]
    Catalog(#[from] litellm_model_catalog::Error),
    #[error(transparent)]
    Tokenizer(#[from] litellm_token_counter::Error),
    #[error("Analysis HTTP client configuration failed")]
    Http(#[from] litellm_http::Error),
    #[error("Analysis JSON data is invalid")]
    Json(#[from] serde_json::Error),
    #[error("Analysis price calculation failed: {0:?}")]
    Cost(litellm_cost::PricingError),
}
