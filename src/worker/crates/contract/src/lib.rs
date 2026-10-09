#![forbid(unsafe_code)]

pub mod auth;
pub mod datasets;
pub mod error;
pub mod eval;
pub mod execution;
pub mod feedback;
pub mod ingestion;
pub mod investigations;
pub mod schema;
pub mod signals;
pub mod worker;

pub use error::ConversionError;

pub const CONTRACT_VERSION: u32 = 1;
pub const CONTRACT_HEADER: &str = "X-Lens-Contract";
pub mod activity;
pub mod github;
