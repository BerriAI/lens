use crate::{
    datasets::validation::{self, Field, Kind as FieldKind},
    error::{DatasetError, ValidationError},
};
use axum::http::HeaderMap;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

#[derive(Clone, Copy)]
pub(crate) enum Model {
    Target,
    Submission,
    Deletion,
    Summary,
    Trace,
}
#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Score,
    Traces,
}
const TRACE_ID: FieldKind = FieldKind::Optional(&FieldKind::String(1, Some(128)));
const SESSION_ID: FieldKind = FieldKind::Optional(&FieldKind::String(1, Some(512)));
const TRACE_REF: FieldKind = FieldKind::String(0, Some(512));
const USER: FieldKind = FieldKind::String(0, Some(256));

impl Model {
    pub(crate) fn fields(self) -> &'static [Field] {
        match self {
            Self::Target => &[
                ("trace_id", false, TRACE_ID),
                ("session_id", false, SESSION_ID),
                ("trace_ref", false, TRACE_REF),
            ],
            Self::Submission => &[
                ("score", true, FieldKind::Feedback(Kind::Score)),
                ("comment", false, FieldKind::String(0, Some(10_000))),
                ("user", false, USER),
                ("trace_id", false, TRACE_ID),
                ("session_id", false, SESSION_ID),
                ("trace_ref", false, TRACE_REF),
            ],
            Self::Deletion => &[
                ("trace_id", false, TRACE_ID),
                ("session_id", false, SESSION_ID),
                ("trace_ref", false, TRACE_REF),
                ("user", false, USER),
            ],
            Self::Summary => &[("traces", true, FieldKind::Feedback(Kind::Traces))],
            Self::Trace => &[
                ("trace_id", true, FieldKind::String(1, Some(128))),
                ("trace_ref", false, TRACE_REF),
            ],
        }
    }
}

pub(crate) fn after(
    value: &Value,
    input: Value,
    model: Model,
    path: &[Value],
    errors: &mut Vec<ValidationError>,
) {
    if !matches!(model, Model::Target | Model::Submission | Model::Deletion) {
        return;
    }
    if value.get("trace_id").is_none_or(Value::is_null)
        == value.get("session_id").is_none_or(Value::is_null)
    {
        errors.push(validation::failure(
            "value_error",
            path,
            "Value error, Pass exactly one of trace_id or session_id",
            input,
            Some(json!({"error":{}})),
        ));
    }
}

pub(crate) fn validate(
    value: &mut Value,
    kind: Kind,
    path: &[Value],
    errors: &mut Vec<ValidationError>,
) {
    match kind {
        Kind::Score => match validation::integer(value, path, true) {
            Ok(integer) if integer.exact().is_none_or(|number| number > 10) => {
                errors.push(validation::failure(
                    "less_than_equal",
                    path,
                    "Input should be less than or equal to 10",
                    value.clone(),
                    Some(json!({"le":10})),
                ))
            }
            Ok(integer) => *value = json!(integer.exact().unwrap()),
            Err(error) => errors.push(error),
        },
        Kind::Traces => {
            let input = value.clone();
            validation::validate(
                value,
                FieldKind::Tuple(
                    &FieldKind::Record(validation::Model::Feedback(Model::Trace)),
                    1,
                ),
                path,
                errors,
            );
            if let Some(length) = value
                .as_array()
                .map(Vec::len)
                .filter(|length| *length > 500)
            {
                errors.push(validation::failure(
                    "too_long",
                    path,
                    format!("Tuple should have at most 500 items after validation, not {length}"),
                    input,
                    Some(json!({"field_type":"Tuple","max_length":500,"actual_length":length})),
                ));
            }
        }
    }
}

pub(crate) fn query<T: DeserializeOwned>(
    raw: Option<&str>,
    model: Model,
) -> Result<T, DatasetError> {
    let mut values: serde_json::Map<String, Value> =
        url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes())
            .map(|(key, value)| (key.into_owned(), json!(value)))
            .collect();
    values.entry("trace_ref").or_insert_with(|| json!(""));
    if matches!(model, Model::Deletion) {
        values.entry("user").or_insert_with(|| json!(""));
    }
    let input = serde_json::to_vec(&values).map_err(DatasetError::Decode)?;
    let parsed = validation::parse_body(&input, &HeaderMap::new())?;
    validation::request(parsed, validation::Model::Feedback(model)).map_err(|error| match error {
        DatasetError::Validation(mut errors) => {
            for error in &mut errors {
                error.value["loc"][0] = json!("query");
            }
            DatasetError::Validation(errors)
        }
        error => error,
    })
}
