#![forbid(unsafe_code)]

pub mod auth;
mod error;
pub mod eval_closer;
mod eval_datasets;
pub mod evals;
pub mod python_routes;
pub mod sessions;

pub use error::{ApiError, EvalApiError, EvalCloserError};

pub fn router() -> axum::Router {
    axum::Router::new()
}
