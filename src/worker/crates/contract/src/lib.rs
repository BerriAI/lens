#![forbid(unsafe_code)]

pub mod auth;
pub mod eval;

pub const CONTRACT_VERSION: u32 = 1;
pub const CONTRACT_HEADER: &str = "X-Lens-Contract";
