mod activity;
mod agent;
mod defaults;
mod enums;
mod findings;
mod investigation;
mod macros;
mod review;
mod strings;

use chrono::{DateTime, Utc};
use macros::{bounded_string, wire_enum};
use std::num::NonZeroU64;

pub use crate::error;
pub use activity::*;
pub use agent::*;
pub use enums::*;
pub use findings::*;
pub use investigation::*;
pub use review::*;
pub use strings::*;

pub const PROTOCOL_VERSION: u64 = 7;

fn validate_length(
    value: &str,
    minimum: usize,
    maximum: Option<usize>,
) -> std::result::Result<(), error::ConversionError> {
    let length = value.chars().count();
    if length < minimum {
        return Err(error::ConversionError::TooShort(minimum));
    }
    if let Some(maximum) = maximum
        && length > maximum
    {
        return Err(error::ConversionError::TooLong(maximum));
    }
    Ok(())
}

fn string_schema(minimum: usize, maximum: Option<usize>) -> schemars::Schema {
    let mut schema = schemars::json_schema!({"type":"string"});
    if minimum > 0 {
        schema.insert("minLength".into(), minimum.into());
    }
    if let Some(maximum) = maximum {
        schema.insert("maxLength".into(), maximum.into());
    }
    schema
}
