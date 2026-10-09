use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to access {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),
    #[error("invalid JWT: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("invalid regular expression: {0}")]
    Regex(#[from] regex::Error),
    #[error("fixture template references unbound value {0}")]
    Unbound(String),
    #[error("fixture capture {binding} was not present in {capture_source}")]
    MissingCapture {
        binding: String,
        capture_source: String,
    },
    #[error("unsupported fixture value at {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum BenchmarkError {
    #[error("benchmark requires {0}")]
    Response(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Url(#[from] url::ParseError),
}
