use crate::{
    datasets::validation::{self, Field, Kind as FieldKind},
    error::ValidationError,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
pub(crate) enum Model {
    Config,
    Signal,
}

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Id,
    Threshold,
}

impl Model {
    pub(crate) fn fields(self) -> &'static [Field] {
        match self {
            Self::Config => &[
                ("model", false, FieldKind::String(0, None)),
                ("threshold", false, FieldKind::Signals(Kind::Threshold)),
                (
                    "signals",
                    false,
                    FieldKind::Tuple(
                        &FieldKind::Record(validation::Model::Signals(Self::Signal)),
                        0,
                    ),
                ),
            ],
            Self::Signal => &[
                ("id", true, FieldKind::Signals(Kind::Id)),
                ("name", true, FieldKind::String(1, None)),
                ("question", true, FieldKind::String(3, None)),
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
    if !matches!(model, Model::Config) {
        return;
    }
    let Some(signals) = value.get("signals").and_then(Value::as_array) else {
        return;
    };
    let message = if signals
        .iter()
        .map(|signal| signal["id"].as_str().unwrap())
        .collect::<BTreeSet<_>>()
        .len()
        != signals.len()
    {
        Some("Signal IDs must be unique")
    } else {
        None
    };
    if let Some(message) = message {
        errors.push(validation::failure(
            "value_error",
            path,
            format!("Value error, {message}"),
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
        Kind::Id => {
            let previous = errors.len();
            validation::validate(value, FieldKind::String(0, None), path, errors);
            if errors.len() != previous {
                return;
            }
            let id = value.as_str().unwrap();
            if id.len() > 64
                || !id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            {
                let pattern = "^[a-z][a-z0-9_]{0,63}$";
                errors.push(validation::failure(
                    "string_pattern_mismatch",
                    path,
                    format!("String should match pattern '{pattern}'"),
                    value.clone(),
                    Some(json!({"pattern":pattern})),
                ));
            }
        }
        Kind::Threshold => {
            if let Err(error) = threshold(value, path) {
                errors.push(error);
            }
        }
    }
}

fn threshold(value: &mut Value, path: &[Value]) -> Result<(), ValidationError> {
    let number = value
        .as_f64()
        .or_else(|| value.as_bool().map(|flag| u8::from(flag) as f64))
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()));
    let Some(number) = number else {
        let (kind, message) = if value.is_string() {
            (
                "float_parsing",
                "Input should be a valid number, unable to parse string as a number",
            )
        } else {
            ("float_type", "Input should be a valid number")
        };
        return Err(validation::failure(
            kind,
            path,
            message,
            value.clone(),
            None,
        ));
    };
    if !number.is_finite() {
        return Err(validation::failure(
            "finite_number",
            path,
            "Input should be a finite number",
            value.clone(),
            None,
        ));
    }
    if number < 0.05 {
        return Err(validation::failure(
            "greater_than_equal",
            path,
            "Input should be greater than or equal to 0.05",
            value.clone(),
            Some(json!({"ge":0.05})),
        ));
    }
    if number > 0.95 {
        return Err(validation::failure(
            "less_than_equal",
            path,
            "Input should be less than or equal to 0.95",
            value.clone(),
            Some(json!({"le":0.95})),
        ));
    }
    *value = json!(number);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Kind, validate};
    use rstest::rstest;
    use serde_json::json;

    #[rstest]
    #[case::length("a".repeat(65))]
    #[case::starts_with_digit("1signal".into())]
    #[case::starts_with_underscore("_signal".into())]
    #[case::uppercase("signalA".into())]
    #[case::punctuation("signal-1".into())]
    #[case::empty(String::new())]
    fn each_invalid_id_rule_is_enforced_independently(#[case] id: String) {
        let mut value = json!(id);
        let mut errors = Vec::new();
        validate(&mut value, Kind::Id, &[json!("id")], &mut errors);
        assert_eq!(errors.len(), 1);
        assert_eq!(
            serde_json::to_value(&errors).unwrap()[0]["type"],
            "string_pattern_mismatch"
        );
    }
}
