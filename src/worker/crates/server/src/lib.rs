#![forbid(unsafe_code)]

pub mod auth;
pub mod datasets;
mod error;
pub mod python_routes;
pub mod sessions;

pub use error::ApiError;

pub fn router() -> axum::Router {
    axum::Router::new()
}
