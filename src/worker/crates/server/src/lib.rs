#![forbid(unsafe_code)]

pub mod auth;
mod error;
pub mod python_routes;

pub use error::ApiError;

pub fn router() -> axum::Router {
    axum::Router::new()
}
