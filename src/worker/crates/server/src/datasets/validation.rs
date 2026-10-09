use axum::http::{HeaderMap, header};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::error::{DatasetError, ValidationError};

#[derive(Clone, Copy)]
pub(crate) enum Model {
    Create,
    Build,
    Revision,
    Case,
    Message,
    Tool,
    CaseSource,
    Trace,
    Finding,
    Text,
    Query,
    IngestionKey,
}

#[derive(Clone, Copy)]
enum Kind {
    String(usize, Option<usize>),
    Integer,
    Boolean,
    Role,
    Tuple(&'static Kind, usize),
    Record(Model),
    Source,
    Tag(&'static str),
    AwareDatetime,
}

const STRING: Kind = Kind::String(0, None);
type Field = (&'static str, bool, Kind);

impl Model {
    fn fields(self) -> &'static [Field] {
        match self {
            Self::Query => &[("sql", true, STRING)],
            Self::IngestionKey => &[
                ("name", false, Kind::String(1, Some(128))),
                ("team_id", false, Kind::String(0, Some(256))),
                ("expires_at", false, Kind::AwareDatetime),
            ],
            Self::Create => &[
                ("name", true, Kind::String(1, Some(120))),
                ("agent_name", false, STRING),
            ],
            Self::Build => &[
                ("sources", true, Kind::Tuple(&Kind::Source, 1)),
                ("dataset_id", false, STRING),
            ],
            Self::Revision => &[
                ("base_revision", true, Kind::Integer),
                ("cases", true, Kind::Tuple(&Kind::Record(Self::Case), 0)),
            ],
            Self::Case => &[
                ("id", true, STRING),
                (
                    "messages",
                    true,
                    Kind::Tuple(&Kind::Record(Self::Message), 0),
                ),
                ("reply", false, STRING),
                (
                    "tool_calls",
                    false,
                    Kind::Tuple(&Kind::Record(Self::Tool), 0),
                ),
                ("expected", false, STRING),
                ("included", false, Kind::Boolean),
                ("source", true, Kind::Record(Self::CaseSource)),
                ("agent_version", false, STRING),
            ],
            Self::Message => &[
                ("role", true, Kind::Role),
                ("content", true, STRING),
                ("name", false, STRING),
                (
                    "tool_calls",
                    false,
                    Kind::Tuple(&Kind::Record(Self::Tool), 0),
                ),
            ],
            Self::Tool => &[("name", true, STRING), ("arguments", true, STRING)],
            Self::CaseSource => &[
                ("trace_id", false, STRING),
                ("trace_ref", false, STRING),
                ("span_id", false, STRING),
                ("finding_id", false, STRING),
                ("lens_id", false, STRING),
            ],
            Self::Trace => &[
                ("kind", false, Kind::Tag("trace")),
                ("trace_id", true, Kind::String(1, None)),
                ("trace_ref", false, STRING),
                ("span_id", false, STRING),
            ],
            Self::Finding => &[
                ("kind", false, Kind::Tag("finding")),
                ("lens_id", true, Kind::String(1, None)),
                ("finding_ids", true, Kind::Tuple(&STRING, 1)),
            ],
            Self::Text => &[
                ("kind", false, Kind::Tag("text")),
                ("text", true, Kind::String(1, None)),
            ],
        }
    }
}

pub(crate) fn failure(
    kind: &str,
    path: &[Value],
    message: impl Into<String>,
    input: Value,
    context: Option<Value>,
) -> ValidationError {
    let mut value = json!({"type": kind, "loc": path, "msg": message.into(), "input": input});
    if let Some(context) = context {
        value["ctx"] = context;
    }
    value.into()
}

fn at(path: &[Value], part: Value) -> Vec<Value> {
    path.iter().cloned().chain(std::iter::once(part)).collect()
}

pub(crate) struct ParsedBody {
    value: Value,
    base_revision: Option<String>,
    original: Option<Box<serde_json::value::RawValue>>,
}

pub(crate) fn parse_body(body: &[u8], headers: &HeaderMap) -> Result<ParsedBody, DatasetError> {
    let json_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| {
            let media = value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            media == "application/json"
                || (media.starts_with("application/") && media.ends_with("+json"))
        });
    if body.is_empty() {
        return Ok(ParsedBody {
            value: Value::Null,
            base_revision: None,
            original: None,
        });
    } else if !json_type {
        return Ok(ParsedBody {
            value: Value::String(String::from_utf8_lossy(body).into_owned()),
            base_revision: None,
            original: None,
        });
    }
    let parsed = jiter::JsonValue::parse(body, false).map_err(|error| {
        let (position, message) = super::json_diagnostics::diagnose(body)
            .unwrap_or_else(|| (error.index, error.to_string()));
        DatasetError::Validation(vec![failure(
            "json_invalid",
            &[json!("body"), json!(position)],
            "JSON decode error",
            json!({}),
            Some(json!({"error": message})),
        )])
    })?;
    let base_revision = match &parsed {
        jiter::JsonValue::Object(fields) => fields
            .iter()
            .rev()
            .find(|(key, _)| key == "base_revision")
            .and_then(|(_, value)| match value {
                jiter::JsonValue::BigInt(integer) => Some(integer.to_string()),
                _ => None,
            }),
        _ => None,
    };
    Ok(ParsedBody {
        value: json_value(&parsed),
        base_revision,
        original: Some(serde_json::from_slice(body).map_err(DatasetError::Decode)?),
    })
}

fn json_value(value: &jiter::JsonValue<'_>) -> Value {
    use jiter::JsonValue;
    match value {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(value) => json!(value),
        JsonValue::Int(value) => json!(value),
        JsonValue::BigInt(value) => {
            serde_json::from_str(&value.to_string()).unwrap_or_else(|_| json!(u64::MAX))
        }
        JsonValue::Float(value) => json!(value),
        JsonValue::Str(value) => json!(value),
        JsonValue::Array(values) => values.iter().map(json_value).collect(),
        JsonValue::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.to_string(), json_value(value)))
                .collect(),
        ),
    }
}

pub(crate) fn request<T: DeserializeOwned>(
    parsed: ParsedBody,
    model: Model,
) -> Result<T, DatasetError> {
    let mut value = parsed.value;
    if let (Model::Revision, Some(revision)) = (model, parsed.base_revision) {
        value["base_revision"] = json!(revision);
    }
    if value.is_null() {
        return Err(DatasetError::Validation(vec![failure(
            "missing",
            &[json!("body")],
            "Field required",
            value,
            None,
        )]));
    }
    let mut errors = Vec::new();
    validate(
        &mut value,
        Kind::Record(model),
        &[json!("body")],
        &mut errors,
    );
    if !errors.is_empty() {
        if let Some(original) = parsed.original {
            for error in &mut errors {
                let path = error.value["loc"].as_array().unwrap();
                let path = if error.value["type"] == "missing" {
                    &path[1..path.len() - 1]
                } else {
                    &path[1..]
                };
                error.input = raw_input(&original, path).map(ToOwned::to_owned);
                if error.value["type"] == "union_tag_invalid"
                    && let Some(input) = &error.input
                    && let Ok(jiter::JsonValue::Object(fields)) =
                        jiter::JsonValue::parse(input.get().as_bytes(), false)
                    && let Some((_, tag)) = fields.iter().rev().find(|(key, _)| key == "kind")
                {
                    let tag = exact_python_repr(tag);
                    error.value["ctx"]["tag"] = json!(tag);
                    error.value["msg"] = json!(format!(
                        "Input tag '{tag}' found using 'kind' does not match any of the expected tags: 'trace', 'finding', 'text'"
                    ));
                }
            }
        }
        return Err(DatasetError::Validation(errors));
    }
    serde_json::from_value(value).map_err(DatasetError::Decode)
}

#[cfg(test)]
fn body<T: DeserializeOwned>(
    body: &[u8],
    headers: &HeaderMap,
    model: Model,
) -> Result<T, DatasetError> {
    request(parse_body(body, headers)?, model)
}

fn raw_input<'a>(
    input: &'a serde_json::value::RawValue,
    path: &[Value],
) -> Option<&'a serde_json::value::RawValue> {
    let Some((first, rest)) = path.split_first() else {
        return Some(input);
    };
    if let Some(index) = first.as_u64() {
        let values: Vec<&serde_json::value::RawValue> = serde_json::from_str(input.get()).ok()?;
        return raw_input(values.get(index as usize)?, rest);
    }
    let key = first.as_str()?;
    let fields: std::collections::HashMap<String, &serde_json::value::RawValue> =
        serde_json::from_str(input.get()).ok()?;
    if let Some(value) = fields.get(key) {
        return raw_input(value, rest);
    }
    let tag: String = serde_json::from_str(fields.get("kind")?.get()).ok()?;
    (tag == key).then(|| raw_input(input, rest)).flatten()
}

fn validate(value: &mut Value, kind: Kind, path: &[Value], errors: &mut Vec<ValidationError>) {
    match kind {
        Kind::AwareDatetime => {
            if value.is_null() {
                return;
            }
            match super::super::ingestion::parse_expiry(value, path) {
                Ok(datetime) => *value = json!(datetime),
                Err(error) => errors.push(error),
            }
        }
        Kind::String(minimum, maximum) => {
            let Some(text) = value.as_str() else {
                errors.push(failure(
                    "string_type",
                    path,
                    "Input should be a valid string",
                    value.clone(),
                    None,
                ));
                return;
            };
            let length = text.chars().count();
            if length < minimum {
                errors.push(failure(
                    "string_too_short",
                    path,
                    format!(
                        "String should have at least {minimum} character{}",
                        if minimum == 1 { "" } else { "s" }
                    ),
                    value.clone(),
                    Some(json!({"min_length": minimum})),
                ));
            } else if let Some(maximum) = maximum.filter(|maximum| length > *maximum) {
                errors.push(failure(
                    "string_too_long",
                    path,
                    format!("String should have at most {maximum} characters"),
                    value.clone(),
                    Some(json!({"max_length": maximum})),
                ));
            }
        }
        Kind::Integer => match integer(value, path, true) {
            Ok(integer) => *value = json!(integer.exact().unwrap_or_default()),
            Err(error) => errors.push(error),
        },
        Kind::Boolean => {
            if let Some(boolean) = boolean(value) {
                *value = json!(boolean);
            } else {
                let (kind, message) = if value.is_string() || value.is_number() {
                    (
                        "bool_parsing",
                        "Input should be a valid boolean, unable to interpret input",
                    )
                } else {
                    ("bool_type", "Input should be a valid boolean")
                };
                errors.push(failure(kind, path, message, value.clone(), None));
            }
        }
        Kind::Role => {
            if !matches!(
                value.as_str(),
                Some("system" | "user" | "assistant" | "tool")
            ) {
                let expected = "'system', 'user', 'assistant' or 'tool'";
                errors.push(failure(
                    "literal_error",
                    path,
                    format!("Input should be {expected}"),
                    value.clone(),
                    Some(json!({"expected":expected})),
                ));
            }
        }
        Kind::Tag(tag) => {
            if value.as_str() != Some(tag) {
                errors.push(failure(
                    "literal_error",
                    path,
                    format!("Input should be '{tag}'"),
                    value.clone(),
                    Some(json!({"expected":format!("'{tag}'")})),
                ));
            }
        }
        Kind::Tuple(item, minimum) => {
            let input = value.clone();
            let Some(items) = value.as_array_mut() else {
                errors.push(failure(
                    "tuple_type",
                    path,
                    "Input should be a valid tuple",
                    input,
                    None,
                ));
                return;
            };
            let mut valid = 0;
            for (index, value) in items.iter_mut().enumerate() {
                let previous = errors.len();
                validate(value, *item, &at(path, json!(index)), errors);
                valid += usize::from(errors.len() == previous);
            }
            if valid < minimum {
                errors.push(failure(
                    "too_short",
                    path,
                    format!(
                        "Tuple should have at least {minimum} item{} after validation, not {valid}",
                        if minimum == 1 { "" } else { "s" }
                    ),
                    input,
                    Some(
                        json!({"field_type":"Tuple", "min_length":minimum, "actual_length":valid}),
                    ),
                ));
            }
        }
        Kind::Record(model) => {
            let input = value.clone();
            let Some(object) = value.as_object_mut() else {
                errors.push(failure(
                    "model_attributes_type",
                    path,
                    "Input should be a valid dictionary or object to extract fields from",
                    input,
                    None,
                ));
                return;
            };
            for (field, required, kind) in model.fields() {
                if let Some(value) = object.get_mut(*field) {
                    validate(value, *kind, &at(path, json!(field)), errors);
                } else if *required {
                    errors.push(failure(
                        "missing",
                        &at(path, json!(field)),
                        "Field required",
                        input.clone(),
                        None,
                    ));
                }
            }
            for (key, value) in object
                .iter()
                .filter(|(key, _)| !model.fields().iter().any(|(name, _, _)| name == key))
            {
                errors.push(failure(
                    "extra_forbidden",
                    &at(path, json!(key)),
                    "Extra inputs are not permitted",
                    value.clone(),
                    None,
                ));
            }
        }
        Kind::Source => {
            if !value.is_object() {
                errors.push(failure(
                    "model_attributes_type",
                    path,
                    "Input should be a valid dictionary or object to extract fields from",
                    value.clone(),
                    None,
                ));
                return;
            }
            let Some(tag) = value.get("kind") else {
                errors.push(failure(
                    "union_tag_not_found",
                    path,
                    "Unable to extract tag using discriminator 'kind'",
                    value.clone(),
                    Some(json!({"discriminator":"'kind'"})),
                ));
                return;
            };
            let model = match tag.as_str() {
                Some("trace") => Model::Trace,
                Some("finding") => Model::Finding,
                Some("text") => Model::Text,
                _ => {
                    let tag = tag
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| python_repr(tag));
                    errors.push(failure("union_tag_invalid", path, format!("Input tag '{tag}' found using 'kind' does not match any of the expected tags: 'trace', 'finding', 'text'"), value.clone(), Some(json!({"discriminator":"'kind'", "tag":tag, "expected_tags":"'trace', 'finding', 'text'"}))));
                    return;
                }
            };
            let path = at(path, tag.clone());
            validate(value, Kind::Record(model), &path, errors);
        }
    }
}

fn boolean(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::Number(number) if number.as_f64() == Some(1.0) => Some(true),
        Value::Number(number) if number.as_f64() == Some(0.0) => Some(false),
        Value::String(text) => match text.to_ascii_lowercase().as_str() {
            "1" | "on" | "t" | "true" | "y" | "yes" => Some(true),
            "0" | "off" | "f" | "false" | "n" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn python_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::String(text) => {
            let quote = if text.contains('\'') && !text.contains('"') {
                '"'
            } else {
                '\''
            };
            let escaped: String = text
                .chars()
                .map(|char| match char {
                    '\\' => "\\\\".to_owned(),
                    '\n' => "\\n".to_owned(),
                    '\r' => "\\r".to_owned(),
                    '\t' => "\\t".to_owned(),
                    char if char == quote => format!("\\{char}"),
                    char if char.is_control() && u32::from(char) < 256 => {
                        format!("\\x{:02x}", u32::from(char))
                    }
                    char => char.to_string(),
                })
                .collect();
            format!("{quote}{escaped}{quote}")
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_repr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!("{}: {}", python_repr(&json!(key)), python_repr(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Number(number) => number.to_string(),
    }
}

fn exact_python_repr(value: &jiter::JsonValue<'_>) -> String {
    match value {
        jiter::JsonValue::BigInt(value) => value.to_string(),
        jiter::JsonValue::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(exact_python_repr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        jiter::JsonValue::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!(
                    "{}: {}",
                    python_repr(&json!(key)),
                    exact_python_repr(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => python_repr(&json_value(value)),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Revision {
    Exact(i64),
    OutsideRange { negative: bool },
}

impl Revision {
    pub(crate) fn exact(self) -> Option<i64> {
        match self {
            Self::Exact(value) => Some(value),
            Self::OutsideRange { .. } => None,
        }
    }
    pub(crate) fn negative(self) -> bool {
        match self {
            Self::Exact(value) => value < 0,
            Self::OutsideRange { negative } => negative,
        }
    }
}

fn integer_text(text: &str) -> Option<Revision> {
    let text = text.trim();
    let bytes = text.as_bytes();
    if !bytes.iter().enumerate().all(|(index, byte)| {
        *byte != b'_'
            || (index > 0
                && bytes.get(index - 1).is_some_and(u8::is_ascii_digit)
                && bytes.get(index + 1).is_some_and(u8::is_ascii_digit))
    }) {
        return None;
    }
    let text = text.replace('_', "");
    let integer = if let Some((integer, fraction)) = text.split_once('.') {
        if fraction.is_empty() || !fraction.bytes().all(|byte| byte == b'0') {
            return None;
        }
        integer
    } else {
        &text
    };
    let digits = integer.strip_prefix(['+', '-']).unwrap_or(integer);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(
        integer
            .parse()
            .map(Revision::Exact)
            .unwrap_or(Revision::OutsideRange {
                negative: integer.starts_with('-'),
            }),
    )
}

pub(crate) fn integer(
    value: &Value,
    path: &[Value],
    nonnegative: bool,
) -> Result<Revision, ValidationError> {
    let parsed = match value {
        Value::Bool(boolean) => Some(Revision::Exact(i64::from(*boolean))),
        Value::Number(number) if !number.to_string().contains(['.', 'e', 'E']) => {
            integer_text(&number.to_string())
        }
        Value::Number(number) => number
            .as_f64()
            .filter(|number| number.is_finite() && number.fract() == 0.0)
            .map(|number| {
                if number >= i64::MIN as f64 && number < i64::MAX as f64 {
                    Revision::Exact(number as i64)
                } else {
                    Revision::OutsideRange {
                        negative: number < 0.0,
                    }
                }
            }),
        Value::String(text) => integer_text(text),
        _ => None,
    };
    let Some(parsed) = parsed else {
        let (kind, message) = match value {
            Value::String(_) => (
                "int_parsing",
                "Input should be a valid integer, unable to parse string as an integer",
            ),
            Value::Number(number)
                if number
                    .as_f64()
                    .is_some_and(|value| value.is_finite() && value.fract() == 0.0) =>
            {
                (
                    "int_parsing_size",
                    "Unable to parse input string as an integer, exceeded maximum size",
                )
            }
            Value::Number(_) => (
                "int_from_float",
                "Input should be a valid integer, got a number with a fractional part",
            ),
            _ => ("int_type", "Input should be a valid integer"),
        };
        return Err(failure(kind, path, message, value.clone(), None));
    };
    if nonnegative && parsed.negative() {
        return Err(failure(
            "greater_than_equal",
            path,
            "Input should be greater than or equal to 0",
            value.clone(),
            Some(json!({"ge":0})),
        ));
    }
    Ok(parsed)
}

pub(super) fn revision_query(query: Option<&str>) -> Result<Option<Revision>, DatasetError> {
    query
        .and_then(|query| {
            url::form_urlencoded::parse(query.as_bytes())
                .filter(|(key, _)| key == "revision")
                .map(|(_, value)| value.into_owned())
                .last()
        })
        .map(|revision| integer(&json!(revision), &[json!("query"), json!("revision")], true))
        .transpose()
        .map_err(|error| DatasetError::Validation(vec![error]))
}

pub(super) fn revision_path(revision: &str) -> Result<Revision, DatasetError> {
    integer(&json!(revision), &[json!("path"), json!("revision")], false)
        .map_err(|error| DatasetError::Validation(vec![error]))
}

pub(super) fn revision_request(
    parsed: ParsedBody,
) -> Result<(lens_contract::datasets::RevisionSave, Revision), DatasetError> {
    let original = parsed
        .base_revision
        .as_ref()
        .map(|revision| json!(revision))
        .or_else(|| parsed.value.get("base_revision").cloned())
        .unwrap_or_default();
    let request = request(parsed, Model::Revision)?;
    let revision = integer(&original, &[json!("body"), json!("base_revision")], true)
        .map_err(|error| DatasetError::Validation(vec![error]))?;
    Ok((request, revision))
}

#[cfg(test)]
mod tests {
    use super::{Model, body, revision_path, revision_query};
    use crate::error::DatasetError;
    use axum::http::HeaderMap;
    use lens_contract::datasets::{DatasetCreate, RevisionSave};
    use rstest::rstest;
    use serde_json::{Value, json};

    #[rstest]
    #[case::missing_name(json!({}), json!([{"type":"missing","loc":["body","name"],"msg":"Field required","input":{}}]))]
    #[case::name_null(json!({"name":null}), json!([{"type":"string_type","loc":["body","name"],"msg":"Input should be a valid string","input":null}]))]
    #[case::empty_name(json!({"name":""}), json!([{"type":"string_too_short","loc":["body","name"],"msg":"String should have at least 1 character","input":"","ctx":{"min_length":1}}]))]
    #[case::extra_field(json!({"name":"ok","other":1}), json!([{"type":"extra_forbidden","loc":["body","other"],"msg":"Extra inputs are not permitted","input":1}]))]
    #[case::nonobject(json!([]), json!([{"type":"model_attributes_type","loc":["body"],"msg":"Input should be a valid dictionary or object to extract fields from","input":[]}]))]
    fn create_validation_preserves_python_errors(#[case] input: Value, #[case] expected: Value) {
        let error = body::<DatasetCreate>(
            &serde_json::to_vec(&input).unwrap(),
            &HeaderMap::new(),
            Model::Create,
        )
        .unwrap_err();
        let DatasetError::Validation(errors) = error else {
            panic!("unexpected error: {error:?}")
        };
        assert_eq!(json!(errors), expected);
    }

    #[rstest]
    #[case::negative_revision(
        Model::Revision,
        r#"{"base_revision":-1000000000000000000000000000001,"cases":[]}"#.to_owned(),
        "greater_than_equal",
        json!(["body", "base_revision"]),
        "-1000000000000000000000000000001".to_owned()
    )]
    #[case::numeric_name(
        Model::Create,
        r#"{"name":1000000000000000000000000000001}"#.to_owned(),
        "string_type",
        json!(["body", "name"]),
        "1000000000000000000000000000001".to_owned()
    )]
    #[case::outside_float_range(
        Model::Create,
        format!("{{\"name\":1{}}}", "0".repeat(310)),
        "string_type",
        json!(["body", "name"]),
        format!("1{}", "0".repeat(310))
    )]
    #[case::nested_source(
        Model::Build,
        r#"{"sources":[{"kind":"trace","trace_id":1000000000000000000000000000001}]}"#.to_owned(),
        "string_type",
        json!(["body", "sources", 0, "trace", "trace_id"]),
        "1000000000000000000000000000001".to_owned()
    )]
    #[tokio::test]
    async fn wide_numeric_validation_inputs_keep_their_exact_json_representation(
        #[case] model: Model,
        #[case] input: String,
        #[case] kind: &str,
        #[case] location: Value,
        #[case] expected_input: String,
    ) {
        use axum::{body::to_bytes, response::IntoResponse};

        #[derive(serde::Deserialize)]
        struct ErrorInput {
            r#type: String,
            loc: Value,
            input: Box<serde_json::value::RawValue>,
        }
        #[derive(serde::Deserialize)]
        struct Errors {
            detail: Vec<ErrorInput>,
        }

        let response = body::<Value>(input.as_bytes(), &HeaderMap::new(), model)
            .unwrap_err()
            .into_response();
        assert_eq!(response.status(), 422);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let errors: Errors = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(errors.detail[0].r#type, kind);
        assert_eq!(errors.detail[0].loc, location);
        assert_eq!(errors.detail[0].input.get(), expected_input);
    }

    #[rstest]
    #[case::none(json!(null), "model_attributes_type", "Input should be a valid dictionary or object to extract fields from")]
    #[case::array(json!([]), "model_attributes_type", "Input should be a valid dictionary or object to extract fields from")]
    #[case::missing_tag(json!({}), "union_tag_not_found", "Unable to extract tag using discriminator 'kind'")]
    fn invalid_build_sources_also_fail_minimum_successful_source_count(
        #[case] source: Value,
        #[case] kind: &str,
        #[case] message: &str,
    ) {
        let input = json!({"sources":[source]});
        let error = body::<Value>(
            &serde_json::to_vec(&input).unwrap(),
            &HeaderMap::new(),
            Model::Build,
        )
        .unwrap_err();
        let DatasetError::Validation(errors) = error else {
            panic!("unexpected error: {error:?}")
        };
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0]["type"], kind);
        assert_eq!(errors[0]["msg"], message);
        assert_eq!(errors[0]["loc"], json!(["body", "sources", 0]));
        assert_eq!(errors[0]["input"], source);
        assert_eq!(
            errors[1],
            json!({"type":"too_short","loc":["body","sources"],"msg":"Tuple should have at least 1 item after validation, not 0","input":[source],"ctx":{"field_type":"Tuple","min_length":1,"actual_length":0}})
        );
    }

    #[rstest]
    fn nested_record_validation_preserves_field_paths() {
        let input = json!({"base_revision":0,"cases":[{"id":"1","messages":[{"role":"invalid","content":""}],"source":{"extra":"forbidden"}}]});
        let error = body::<RevisionSave>(
            &serde_json::to_vec(&input).unwrap(),
            &HeaderMap::new(),
            Model::Revision,
        )
        .unwrap_err();
        let DatasetError::Validation(errors) = error else {
            panic!("unexpected error: {error:?}")
        };
        assert_eq!(
            json!(errors),
            json!([
                {"type":"literal_error","loc":["body","cases",0,"messages",0,"role"],"msg":"Input should be 'system', 'user', 'assistant' or 'tool'","input":"invalid","ctx":{"expected":"'system', 'user', 'assistant' or 'tool'"}},
                {"type":"extra_forbidden","loc":["body","cases",0,"source","extra"],"msg":"Extra inputs are not permitted","input":"forbidden"}
            ])
        );
    }

    #[rstest]
    #[case::integer_string(json!("1_000"), json!("off"), 1000, false)]
    #[case::integral_decimal(json!("1.00"), json!(1), 1, true)]
    #[case::integral_float(json!(1.0), json!("YES"), 1, true)]
    #[case::boolean_integer(json!(false), json!(0.0), 0, false)]
    fn supported_pydantic_coercions_are_preserved(
        #[case] revision: Value,
        #[case] included: Value,
        #[case] expected_revision: i64,
        #[case] expected_included: bool,
    ) {
        let input = json!({"base_revision":revision,"cases":[{"id":"x","messages":[],"included":included,"source":{}}]});
        let request = body::<RevisionSave>(
            &serde_json::to_vec(&input).unwrap(),
            &HeaderMap::new(),
            Model::Revision,
        )
        .unwrap();
        assert_eq!(request.base_revision, expected_revision);
        assert_eq!(request.cases[0].included, expected_included);
        assert_eq!(request.cases[0].reply, "");
        assert!(request.cases[0].tool_calls.is_empty());
    }

    #[rstest]
    #[case::unicode("é".repeat(120), true)]
    #[case::overlong("é".repeat(121), false)]
    fn name_length_counts_unicode_characters(#[case] name: String, #[case] valid: bool) {
        let input = json!({"name":name});
        let result = body::<DatasetCreate>(
            &serde_json::to_vec(&input).unwrap(),
            &HeaderMap::new(),
            Model::Create,
        );
        assert_eq!(result.is_ok(), valid);
        if let Err(DatasetError::Validation(errors)) = result {
            assert_eq!(
                errors,
                vec![
                    json!({"type":"string_too_long","loc":["body","name"],"msg":"String should have at most 120 characters","input":name,"ctx":{"max_length":120}})
                ]
            );
        }
    }

    #[rstest]
    #[case::missing(None, None)]
    #[case::unrelated(Some("other=3"), None)]
    #[case::last_duplicate(Some("revision=1&revision=2"), Some(2))]
    #[case::decimal(Some("revision=1.00"), Some(1))]
    #[case::encoded_plus(Some("revision=%2B1"), Some(1))]
    fn revision_query_matches_fastapi_coercion(
        #[case] query: Option<&str>,
        #[case] expected: Option<i64>,
    ) {
        assert_eq!(
            revision_query(query)
                .unwrap()
                .map(|revision| revision.exact()),
            expected.map(Some)
        );
    }

    #[rstest]
    fn negative_revision_is_rejected_only_for_query() {
        assert_eq!(revision_path("-1").unwrap().exact(), Some(-1));
        let error = revision_query(Some("revision=-1")).unwrap_err();
        let DatasetError::Validation(errors) = error else {
            panic!("unexpected error: {error:?}")
        };
        assert_eq!(
            errors,
            vec![
                json!({"type":"greater_than_equal","loc":["query","revision"],"msg":"Input should be greater than or equal to 0","input":"-1","ctx":{"ge":0}})
            ]
        );
    }

    #[rstest]
    #[case::null(json!(null), "None")]
    #[case::true_value(json!(true), "True")]
    #[case::false_value(json!(false), "False")]
    #[case::array(json!([null,"a",true]), "[None, 'a', True]")]
    #[case::object(json!({"a":false}), "{'a': False}")]
    fn invalid_discriminator_uses_python_display(#[case] tag: Value, #[case] display: &str) {
        let source = json!({"kind":tag});
        let error = body::<Value>(
            &serde_json::to_vec(&json!({"sources":[source]})).unwrap(),
            &HeaderMap::new(),
            Model::Build,
        )
        .unwrap_err();
        let DatasetError::Validation(errors) = error else {
            panic!("unexpected error: {error:?}")
        };
        assert_eq!(
            errors[0],
            json!({"type":"union_tag_invalid","loc":["body","sources",0],
            "msg":format!("Input tag '{display}' found using 'kind' does not match any of the expected tags: 'trace', 'finding', 'text'"),
            "input":source,"ctx":{"discriminator":"'kind'","tag":display,"expected_tags":"'trace', 'finding', 'text'"}})
        );
    }

    #[rstest]
    #[case::number("1000000000000000000000000000001", "1000000000000000000000000000001")]
    #[case::array(
        "[1000000000000000000000000000001]",
        "[1000000000000000000000000000001]"
    )]
    #[case::object(
        "{\"number\":1000000000000000000000000000001}",
        "{'number': 1000000000000000000000000000001}"
    )]
    fn invalid_discriminator_preserves_large_integer_display(
        #[case] tag: &str,
        #[case] display: &str,
    ) {
        let input = format!("{{\"sources\":[{{\"kind\":{tag}}}]}}");
        let DatasetError::Validation(errors) =
            body::<Value>(input.as_bytes(), &HeaderMap::new(), Model::Build).unwrap_err()
        else {
            panic!("expected validation errors");
        };
        assert_eq!(errors[0]["type"], "union_tag_invalid");
        assert_eq!(errors[0]["ctx"]["tag"], display);
        assert_eq!(
            errors[0]["msg"],
            format!(
                "Input tag '{display}' found using 'kind' does not match any of the expected tags: 'trace', 'finding', 'text'"
            )
        );
    }
}
