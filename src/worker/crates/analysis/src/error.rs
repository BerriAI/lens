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
    #[error("Analysis provider returned HTTP {status}: {diagnostic}")]
    Provider {
        status: u16,
        retry_after: Option<u64>,
        diagnostic: ProviderDiagnostic,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFailureKind {
    Authentication,
    PermissionDenied,
    RateLimited,
    BudgetExceeded,
    ContextExceeded,
    ContentRejected,
    InvalidRequest,
    Unavailable,
    Timeout,
    Other,
}

impl std::fmt::Display for ProviderFailureKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Authentication => "provider authentication failed",
            Self::PermissionDenied => "provider permission denied",
            Self::RateLimited => "provider rate limit exceeded",
            Self::BudgetExceeded => "provider budget or quota exceeded",
            Self::ContextExceeded => "provider context limit exceeded",
            Self::ContentRejected => "provider content policy rejected the request",
            Self::InvalidRequest => "provider rejected an invalid request",
            Self::Unavailable => "provider unavailable",
            Self::Timeout => "provider request timed out",
            Self::Other => "provider rejected the request",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDiagnostic {
    pub classification: ProviderFailureKind,
    pub request_id: Option<String>,
}

impl ProviderDiagnostic {
    pub(crate) fn from_response(
        status: u16,
        headers: &reqwest::header::HeaderMap,
        body: &[u8],
    ) -> Self {
        let body = serde_json::from_slice::<serde_json::Value>(body).ok();
        let classification = body
            .as_ref()
            .and_then(|body| {
                ["/error/code", "/error/type"]
                    .into_iter()
                    .filter_map(|path| body.pointer(path).and_then(serde_json::Value::as_str))
                    .find_map(provider_failure_kind)
            })
            .unwrap_or(match status {
                401 => ProviderFailureKind::Authentication,
                403 => ProviderFailureKind::PermissionDenied,
                402 => ProviderFailureKind::BudgetExceeded,
                408 | 504 => ProviderFailureKind::Timeout,
                429 => ProviderFailureKind::RateLimited,
                400..=499 => ProviderFailureKind::InvalidRequest,
                500..=599 => ProviderFailureKind::Unavailable,
                _ => ProviderFailureKind::Other,
            });
        let request_id = ["x-request-id", "request-id", "x-litellm-call-id"]
            .into_iter()
            .filter_map(|name| headers.get(name).and_then(|value| value.to_str().ok()))
            .find_map(provider_request_id);
        Self {
            classification,
            request_id,
        }
    }
}

impl std::fmt::Display for ProviderDiagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.classification.fmt(formatter)?;
        if let Some(request_id) = &self.request_id {
            write!(formatter, " (request ID: {request_id})")?;
        }
        Ok(())
    }
}

fn provider_failure_kind(value: &str) -> Option<ProviderFailureKind> {
    match value {
        "authentication_error" | "invalid_api_key" | "AuthenticationError" => {
            Some(ProviderFailureKind::Authentication)
        }
        "permission_error" | "permission_denied" | "PermissionDeniedError" => {
            Some(ProviderFailureKind::PermissionDenied)
        }
        "rate_limit_error" | "rate_limit_exceeded" | "RateLimitError" => {
            Some(ProviderFailureKind::RateLimited)
        }
        "budget_exceeded"
        | "budget_exceeded_error"
        | "insufficient_quota"
        | "BudgetExceededError" => Some(ProviderFailureKind::BudgetExceeded),
        "context_length_exceeded"
        | "context_window_exceeded"
        | "prompt_too_long"
        | "ContextWindowExceededError" => Some(ProviderFailureKind::ContextExceeded),
        "content_filter" | "content_filter_error" | "content_policy_violation" => {
            Some(ProviderFailureKind::ContentRejected)
        }
        "invalid_request_error" | "bad_request_error" | "BadRequestError" => {
            Some(ProviderFailureKind::InvalidRequest)
        }
        "overloaded_error"
        | "service_unavailable_error"
        | "server_error"
        | "internal_server_error"
        | "ServiceUnavailableError"
        | "InternalServerError" => Some(ProviderFailureKind::Unavailable),
        "timeout" | "timeout_error" | "Timeout" | "APITimeoutError" => {
            Some(ProviderFailureKind::Timeout)
        }
        _ => None,
    }
}

fn provider_request_id(value: &str) -> Option<String> {
    if let Ok(id) = uuid::Uuid::parse_str(value) {
        return Some(id.to_string());
    }
    value
        .strip_prefix("req_")
        .filter(|suffix| {
            (8..=100).contains(&suffix.len())
                && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        .map(|_| value.to_owned())
}
