use lens_inference::{ModelCapacity, OutputLimits};
use std::{fmt, time::Duration};
use url::Url;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Provider {
    OpenAi,
    Anthropic,
    OpenAiCompatible,
}

#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Clone, Debug)]
pub struct Deployment {
    pub name: String,
    pub model: String,
    pub provider: Provider,
    pub api_base: Option<Url>,
    pub api_key: Secret,
    pub input_cost_per_token: Option<f64>,
    pub output_cost_per_token: Option<f64>,
    pub capacity: ModelCapacity,
    pub output_limits: OutputLimits,
}

#[derive(Clone, Copy, Debug)]
pub struct TransportLimits {
    pub timeout: Duration,
    pub max_response_bytes: usize,
}

impl Default for TransportLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(600),
            max_response_bytes: usize::MAX,
        }
    }
}
