use std::{fmt, time::Duration};

use serde::Deserialize;
use url::Url;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum Provider {
    #[serde(rename = "typesafe")]
    Typesafe,
    #[serde(rename = "perplexity")]
    Perplexity,
    #[serde(rename = "openrouter")]
    OpenRouter,
    #[serde(rename = "cloudflare")]
    Cloudflare,
    #[serde(rename = "strands_decider")]
    StrandsDecider,
    #[serde(rename = "decisions_compatible")]
    DecisionsCompatible,
}

impl Provider {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Typesafe => "typesafe",
            Self::Perplexity => "perplexity",
            Self::OpenRouter => "openrouter",
            Self::Cloudflare => "cloudflare",
            Self::StrandsDecider => "strands_decider",
            Self::DecisionsCompatible => "decisions_compatible",
        }
    }
}

#[derive(Clone)]
pub struct Secret(pub(crate) String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
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
    pub api_key: Option<Secret>,
}

#[derive(Clone, Copy, Debug)]
pub struct TransportLimits {
    pub timeout: Duration,
    pub max_response_bytes: usize,
}

impl Default for TransportLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            max_response_bytes: 1024 * 1024,
        }
    }
}
