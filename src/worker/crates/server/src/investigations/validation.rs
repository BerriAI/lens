use chrono::{Datelike, TimeDelta, Utc};
use lens_contract::{investigations::RunRequest, worker::LensSettings};
use serde_json::{Value, json};

use crate::{
    datasets::validation::{self, Field, Kind as FieldKind},
    error::{DatasetError, ValidationError},
};

#[derive(Clone, Copy)]
pub(crate) enum Model {
    Settings,
    Check,
    Filter,
    Run,
    Finding,
}
#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Literal(&'static [&'static str]),
    PositiveInteger,
    Lookback,
    Interval,
    Number(Option<f64>),
    Datetime,
}

const STRING: FieldKind = FieldKind::String(0, None);
const POSITIVE: FieldKind = FieldKind::Investigation(Kind::PositiveInteger);
const LOOKBACK: FieldKind = FieldKind::Investigation(Kind::Lookback);
const DATETIME: FieldKind = FieldKind::Investigation(Kind::Datetime);
const SETTINGS: FieldKind = FieldKind::Record(validation::Model::Investigation(Model::Settings));

impl Model {
    pub(crate) fn fields(self) -> &'static [Field] {
        match self {
            Self::Settings => &[
                (
                    "source",
                    false,
                    FieldKind::Investigation(Kind::Literal(&["traces", "requests", "both"])),
                ),
                ("service", false, STRING),
                ("agent_name", false, STRING),
                (
                    "filters",
                    false,
                    FieldKind::Tuple(
                        &FieldKind::Record(validation::Model::Investigation(Self::Filter)),
                        0,
                    ),
                ),
                ("sample_size", false, FieldKind::Optional(&POSITIVE)),
                (
                    "sample_percent",
                    false,
                    FieldKind::Investigation(Kind::Number(Some(100.0))),
                ),
                ("team_id", false, STRING),
                ("execution_ids", false, FieldKind::Tuple(&STRING, 0)),
                ("name", true, FieldKind::String(1, None)),
                ("context", false, STRING),
                ("lookback_hours", false, LOOKBACK),
                (
                    "checks",
                    false,
                    FieldKind::Tuple(
                        &FieldKind::Record(validation::Model::Investigation(Self::Check)),
                        0,
                    ),
                ),
                ("model", true, FieldKind::String(1, None)),
                ("enabled", false, FieldKind::Boolean),
                (
                    "interval_minutes",
                    false,
                    FieldKind::Investigation(Kind::Interval),
                ),
                ("concurrency", false, POSITIVE),
                (
                    "monthly_budget",
                    false,
                    FieldKind::Investigation(Kind::Number(None)),
                ),
            ],
            Self::Check => &[
                ("id", true, FieldKind::String(1, None)),
                ("instruction", true, FieldKind::String(3, None)),
                ("enabled", false, FieldKind::Boolean),
            ],
            Self::Filter => &[
                ("key", true, FieldKind::String(1, None)),
                ("value", true, FieldKind::String(1, None)),
            ],
            Self::Run => &[
                ("settings", false, FieldKind::Optional(&SETTINGS)),
                ("lookback_hours", false, FieldKind::Optional(&LOOKBACK)),
                ("start", false, FieldKind::Optional(&DATETIME)),
                ("end", false, FieldKind::Optional(&DATETIME)),
                (
                    "agent_name",
                    false,
                    FieldKind::Optional(&FieldKind::String(0, Some(200))),
                ),
            ],
            Self::Finding => &[
                (
                    "status",
                    true,
                    FieldKind::Investigation(Kind::Literal(&["open", "resolved", "dismissed"])),
                ),
                ("reason", false, STRING),
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
    let result = match model {
        Model::Settings => serde_json::from_value::<LensSettings>(value.clone())
            .ok()
            .map(|settings| lens_investigations::validate_settings(&settings, Utc::now())),
        Model::Run => serde_json::from_value::<RunRequest>(value.clone())
            .ok()
            .map(|request| lens_investigations::validate_run(&request, Utc::now())),
        _ => None,
    };
    if let Some(Err(error)) = result {
        errors.push(value_error(path, input, &error.to_string()));
    }
}

fn value_error(path: &[Value], input: Value, reason: &str) -> ValidationError {
    validation::failure(
        "value_error",
        path,
        format!("Value error, {reason}"),
        input,
        Some(json!({"error":{}})),
    )
}

pub(crate) fn validate(
    value: &mut Value,
    kind: Kind,
    path: &[Value],
    errors: &mut Vec<ValidationError>,
) {
    let result = match kind {
        Kind::Literal(choices) => literal(value, choices, path),
        Kind::PositiveInteger | Kind::Lookback | Kind::Interval => positive(value, kind, path),
        Kind::Number(maximum) => number(value, maximum, path),
        Kind::Datetime => datetime(value, path),
    };
    if let Err(error) = result {
        errors.push(error);
    }
}

fn literal(value: &Value, choices: &[&str], path: &[Value]) -> Result<(), ValidationError> {
    if value.as_str().is_some_and(|text| choices.contains(&text)) {
        return Ok(());
    }
    let quoted: Vec<_> = choices.iter().map(|choice| format!("'{choice}'")).collect();
    let expected = format!(
        "{} or {}",
        quoted[..quoted.len() - 1].join(", "),
        quoted.last().unwrap()
    );
    Err(validation::failure(
        "literal_error",
        path,
        format!("Input should be {expected}"),
        value.clone(),
        Some(json!({"expected":expected})),
    ))
}

fn positive(value: &mut Value, kind: Kind, path: &[Value]) -> Result<(), ValidationError> {
    let integer = validation::integer(value, path, false)?;
    if integer.negative() || integer.exact() == Some(0) {
        return Err(validation::failure(
            "greater_than_equal",
            path,
            "Input should be greater than or equal to 1",
            value.clone(),
            Some(json!({"ge":1})),
        ));
    }
    let now = Utc::now();
    let calendar = match kind {
        Kind::Lookback => Some((
            integer
                .exact()
                .and_then(TimeDelta::try_hours)
                .and_then(|span| now.checked_sub_signed(span)),
            "Lookback",
        )),
        Kind::Interval => Some((
            integer
                .exact()
                .and_then(TimeDelta::try_minutes)
                .and_then(|span| now.checked_add_signed(span)),
            "Interval",
        )),
        _ => None,
    };
    if let Some((date, name)) = calendar
        && date.is_none_or(|date| !(1..=9999).contains(&date.year()))
    {
        return Err(value_error(
            path,
            value.clone(),
            &format!("{name} exceeds the supported calendar range"),
        ));
    }
    let numeric = integer
        .exact()
        .map(|value| value as u64)
        .or_else(|| value.as_u64())
        .unwrap_or(u64::MAX);
    *value = json!(numeric);
    Ok(())
}

fn number(value: &mut Value, maximum: Option<f64>, path: &[Value]) -> Result<(), ValidationError> {
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
    if number <= 0.0 {
        return Err(validation::failure(
            "greater_than",
            path,
            "Input should be greater than 0",
            value.clone(),
            Some(json!({"gt":0.0})),
        ));
    }
    if let Some(maximum) = maximum.filter(|maximum| number > *maximum) {
        return Err(validation::failure(
            "less_than_equal",
            path,
            format!("Input should be less than or equal to {maximum}"),
            value.clone(),
            Some(json!({"le":maximum})),
        ));
    }
    *value = json!(number);
    Ok(())
}

fn datetime(value: &mut Value, path: &[Value]) -> Result<(), ValidationError> {
    let date = match crate::ingestion::parse_expiry(value, path) {
        Ok(date) => date,
        Err(error) if error.value["type"] == "timezone_aware" => {
            let text = value.as_str().unwrap();
            let normalized = if text.len() == 10 {
                format!("{text}T00:00:00Z")
            } else {
                format!("{text}Z")
            };
            crate::ingestion::parse_expiry(&json!(normalized), path)?
        }
        Err(error) => return Err(error),
    };
    *value = json!(date);
    Ok(())
}

pub(crate) fn offset(query: Option<&str>, name: &str) -> Result<u64, DatasetError> {
    let Some((_, value)) = query.and_then(|query| {
        url::form_urlencoded::parse(query.as_bytes())
            .filter(|(key, _)| key == name)
            .last()
    }) else {
        return Ok(0);
    };
    let input = json!(value);
    let path = [json!("query"), json!(name)];
    let integer = validation::integer(&input, &path, false)
        .map_err(|error| DatasetError::Validation(vec![error]))?;
    if integer.negative() {
        return Err(DatasetError::Validation(vec![validation::failure(
            "greater_than_equal",
            &path,
            "Input should be greater than or equal to 0",
            input,
            Some(json!({"ge":0})),
        )]));
    }
    Ok(integer
        .exact()
        .map(|number| number as u64)
        .unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::zero(Kind::PositiveInteger,json!(0),"greater_than_equal")]
    #[case::negative(Kind::PositiveInteger,json!(-1),"greater_than_equal")]
    #[case::fraction(Kind::PositiveInteger,json!(1.5),"int_from_float")]
    #[case::integer_text(Kind::PositiveInteger,json!("bad"),"int_parsing")]
    #[case::percent_maximum(Kind::Number(Some(100.0)),json!(100.1),"less_than_equal")]
    #[case::zero_budget(Kind::Number(None),json!(0),"greater_than")]
    #[case::nan(Kind::Number(None),json!("NaN"),"finite_number")]
    #[case::infinite(Kind::Number(None),json!("Infinity"),"finite_number")]
    #[case::null_number(Kind::Number(None), Value::Null, "float_type")]
    #[case::invalid_number(Kind::Number(None),json!("bad"),"float_parsing")]
    #[case::lookback(Kind::Lookback,json!(i64::MAX),"value_error")]
    #[case::interval(Kind::Interval,json!(i64::MAX),"value_error")]
    #[case::invalid_datetime(Kind::Datetime,json!(true),"datetime_type")]
    #[case::invalid_source(Kind::Literal(&["traces","requests","both"]),json!("logs"),"literal_error")]
    fn rejected_values_preserve_the_field_and_original_input(
        #[case] kind: Kind,
        #[case] input: Value,
        #[case] error_type: &str,
    ) {
        let mut value = input.clone();
        let mut errors = Vec::new();
        validate(
            &mut value,
            kind,
            &[json!("body"), json!("field")],
            &mut errors,
        );
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["type"], error_type);
        assert_eq!(errors[0]["input"], input);
        assert_eq!(errors[0]["loc"], json!(["body", "field"]));
    }

    #[rstest]
    #[case::minimum_integer(Kind::PositiveInteger,json!(1),json!(1))]
    #[case::integer_coercion(Kind::PositiveInteger,json!("12.0"),json!(12))]
    #[case::boolean_integer(Kind::PositiveInteger,json!(true),json!(1))]
    #[case::minimum_lookback(Kind::Lookback,json!(1),json!(1))]
    #[case::minimum_interval(Kind::Interval,json!(1),json!(1))]
    #[case::hundred_percent(Kind::Number(Some(100.0)),json!(100),json!(100.0))]
    #[case::fractional_percent(Kind::Number(Some(100.0)),json!(0.25),json!(0.25))]
    #[case::numeric_text(Kind::Number(None),json!(" 25.5 "),json!(25.5))]
    #[case::boolean_float(Kind::Number(None),json!(true),json!(1.0))]
    #[case::naive_date(Kind::Datetime,json!("2026-01-01"),json!("2026-01-01T00:00:00Z"))]
    #[case::naive_datetime(Kind::Datetime,json!("2026-01-01T01:02:03"),json!("2026-01-01T01:02:03Z"))]
    #[case::timezone(Kind::Datetime,json!("2026-01-01T01:02:03+01:00"),json!("2026-01-01T00:02:03Z"))]
    #[case::request_source(Kind::Literal(&["traces","requests","both"]),json!("requests"),json!("requests"))]
    fn valid_inputs_keep_pydantic_coercions(
        #[case] kind: Kind,
        #[case] input: Value,
        #[case] expected: Value,
    ) {
        let mut value = input;
        let mut errors = Vec::new();
        validate(&mut value, kind, &[], &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(value, expected);
    }

    #[rstest]
    #[case::absent(None, 0)]
    #[case::last_parameter(Some("offset=4&offset=7"), 7)]
    #[case::large(Some("offset=18446744073709551616"), u64::MAX)]
    fn offset_uses_the_last_valid_nonnegative_integer(
        #[case] query: Option<&str>,
        #[case] expected: u64,
    ) {
        assert_eq!(offset(query, "offset").unwrap(), expected);
    }
}
