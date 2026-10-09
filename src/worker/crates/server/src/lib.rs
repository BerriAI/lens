#![forbid(unsafe_code)]

pub mod auth;
pub mod datasets;
mod error;
pub mod eval_closer;
mod eval_datasets;
pub mod evals;
pub mod feedback;
pub mod ingestion;
pub mod investigations;
pub mod routing;
pub mod service;
pub mod sessions;
pub mod signals;
pub mod tracing;
pub mod ui;

pub use error::{ApiError, EvalApiError, EvalCloserError};

pub fn router() -> axum::Router {
    axum::Router::new()
}
pub mod activity;
pub mod models;
