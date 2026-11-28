#![forbid(unsafe_code)]

pub mod agent_io;
pub mod auth;
pub mod datasets;
pub mod error;
pub mod eval;
pub mod execution;
pub mod feedback;
pub mod ingestion;
pub mod investigations;
pub mod mcp;
pub mod optimizations;
pub mod profiles;
pub mod schema;
pub mod signal_alerts;
pub mod signals;
pub mod worker;

pub use error::{ConversionError, InvalidAgentIo, InvalidCaseResult};

pub const CONTRACT_VERSION: u32 = 1;
pub const AGENT_IO_CONTRACT_VERSION: u32 = 2;
pub const CONTRACT_HEADER: &str = "X-Lens-Contract";
pub mod activity;
pub mod alert_rules;
pub mod github;
