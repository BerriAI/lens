#![forbid(unsafe_code)]

pub mod auth;
pub mod datasets;
mod error;
pub mod eval_closer;
mod eval_datasets;
pub mod evals;
pub mod ingestion;
pub mod python_routes;
pub mod service;
pub mod sessions;
pub mod tracing;
pub mod ui;

pub use error::{ApiError, EvalApiError, EvalCloserError};

pub fn router() -> axum::Router {
    axum::Router::new()
}
