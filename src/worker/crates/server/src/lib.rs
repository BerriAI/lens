#![forbid(unsafe_code)]

pub mod auth;
pub mod datasets;
mod error;
pub mod ingestion;
pub mod python_routes;
pub mod service;
pub mod sessions;
pub mod tracing;
pub mod ui;

pub use error::ApiError;

pub fn router() -> axum::Router {
    axum::Router::new()
}
