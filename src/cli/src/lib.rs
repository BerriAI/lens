#![forbid(unsafe_code)]

pub mod arguments;
pub mod demo;
mod error;
pub mod local;
pub mod setup;

#[cfg(feature = "python")]
pub mod python;

pub use error::{Error, Result};
