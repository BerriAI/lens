macro_rules! bounded_string {
    ($name:ident, $min:expr, $max:expr $(, default = $default:literal)?) => {
        #[derive(serde::Serialize, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #[serde(transparent)]
        pub struct $name(String);
        impl std::ops::Deref for $name {
            type Target = String;
            fn deref(&self) -> &String { &self.0 }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self { value.0 }
        }
        impl std::str::FromStr for $name {
            type Err = crate::error::ConversionError;
            fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
                super::validate_length(value, $min, $max)?;
                Ok(Self(value.to_owned()))
            }
        }
        impl TryFrom<&str> for $name {
            type Error = crate::error::ConversionError;
            fn try_from(value: &str) -> std::result::Result<Self, crate::error::ConversionError> { value.parse() }
        }
        impl TryFrom<&String> for $name {
            type Error = crate::error::ConversionError;
            fn try_from(value: &String) -> std::result::Result<Self, crate::error::ConversionError> { value.parse() }
        }
        impl TryFrom<String> for $name {
            type Error = crate::error::ConversionError;
            fn try_from(value: String) -> std::result::Result<Self, crate::error::ConversionError> {
                super::validate_length(&value, $min, $max)?;
                Ok(Self(value))
            }
        }
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
                let value = <String as serde::Deserialize>::deserialize(deserializer)?;
                value.try_into().map_err(serde::de::Error::custom)
            }
        }
        impl schemars::JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> { stringify!($name).into() }
            fn inline_schema() -> bool { true }
            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                super::string_schema($min, $max)
            }
        }
        $(impl Default for $name { fn default() -> Self { Self($default.to_owned()) } })?
    };
}

macro_rules! wire_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? } $(, default = $default:ident)?) => {
        #[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema, Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #[schemars(inline)]
        pub enum $name { $(#[serde(rename = $value)] $variant),+ }
        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(match self { $(Self::$variant => $value),+ })
            }
        }
        impl std::str::FromStr for $name {
            type Err = crate::error::ConversionError;
            fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
                match value { $($value => Ok(Self::$variant)),+, _ => Err(crate::error::ConversionError::InvalidValue) }
            }
        }
        impl TryFrom<&str> for $name {
            type Error = crate::error::ConversionError;
            fn try_from(value: &str) -> std::result::Result<Self, crate::error::ConversionError> { value.parse() }
        }
        impl TryFrom<&String> for $name {
            type Error = crate::error::ConversionError;
            fn try_from(value: &String) -> std::result::Result<Self, crate::error::ConversionError> { value.parse() }
        }
        impl TryFrom<String> for $name {
            type Error = crate::error::ConversionError;
            fn try_from(value: String) -> std::result::Result<Self, crate::error::ConversionError> { value.parse() }
        }
        $(impl Default for $name { fn default() -> Self { Self::$default } })?
    };
}

pub(super) use {bounded_string, wire_enum};
