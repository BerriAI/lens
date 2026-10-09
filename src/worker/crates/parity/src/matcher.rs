use std::collections::BTreeMap;

use regex::RegexBuilder;
use serde_json::Value;
use similar::TextDiff;
use uuid::Uuid;

use crate::error::Result;
use crate::model::{Mismatch, ResponseFixture};

pub(crate) fn response_mismatches(
    expected: &ResponseFixture,
    actual: &ResponseFixture,
    bindings: &mut BTreeMap<String, String>,
) -> Result<Vec<Mismatch>> {
    let mut mismatches = Vec::new();
    compare(
        &Value::from(expected.status),
        &Value::from(actual.status),
        "$.status",
        bindings,
        &mut mismatches,
    )?;
    compare(
        &serde_json::to_value(&expected.headers).expect("headers serialize"),
        &serde_json::to_value(&actual.headers).expect("headers serialize"),
        "$.headers",
        bindings,
        &mut mismatches,
    )?;
    compare(
        &expected.body,
        &actual.body,
        "$.body",
        bindings,
        &mut mismatches,
    )?;
    Ok(mismatches)
}

fn compare(
    expected: &Value,
    actual: &Value,
    path: &str,
    bindings: &mut BTreeMap<String, String>,
    mismatches: &mut Vec<Mismatch>,
) -> Result<()> {
    match (expected, actual) {
        (Value::String(expected), Value::String(actual)) => {
            match_string(expected, actual, path, bindings, mismatches)?;
        }
        (Value::Object(expected), Value::Object(actual)) => {
            for key in expected
                .keys()
                .chain(actual.keys().filter(|key| !expected.contains_key(*key)))
            {
                let child_path = format!("{path}{}", path_key(key));
                match (expected.get(key), actual.get(key)) {
                    (Some(expected), Some(actual)) => {
                        compare(expected, actual, &child_path, bindings, mismatches)?;
                    }
                    (Some(expected), None) => mismatch(
                        mismatches,
                        child_path,
                        format_value(expected),
                        "<missing>".to_owned(),
                    ),
                    (None, Some(actual)) => mismatch(
                        mismatches,
                        child_path,
                        "<missing>".to_owned(),
                        format_value(actual),
                    ),
                    (None, None) => unreachable!(),
                }
            }
        }
        (Value::Array(expected), Value::Array(actual)) => {
            if expected.len() != actual.len() {
                mismatch(
                    mismatches,
                    format!("{path}.length"),
                    expected.len().to_string(),
                    actual.len().to_string(),
                );
            }
            for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
                compare(
                    expected,
                    actual,
                    &format!("{path}[{index}]"),
                    bindings,
                    mismatches,
                )?;
            }
        }
        _ if expected == actual => {}
        _ => mismatch(
            mismatches,
            path.to_owned(),
            format_value(expected),
            format_value(actual),
        ),
    }
    Ok(())
}

fn path_key(key: &str) -> String {
    if key
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        format!(".{key}")
    } else {
        format!(
            "[{}]",
            serde_json::to_string(key).expect("string serializes")
        )
    }
}

fn mismatch(mismatches: &mut Vec<Mismatch>, path: String, expected: String, actual: String) {
    mismatches.push(Mismatch {
        path,
        expected,
        actual,
    });
}

fn format_value(value: &Value) -> String {
    serde_json::to_string(value).expect("JSON value serializes")
}

fn match_string(
    expected: &str,
    actual: &str,
    path: &str,
    bindings: &mut BTreeMap<String, String>,
    mismatches: &mut Vec<Mismatch>,
) -> Result<()> {
    let Some(parts) = pattern_parts(expected) else {
        if expected != actual {
            mismatch(
                mismatches,
                path.to_owned(),
                format!("{expected:?}"),
                format!("{actual:?}"),
            );
        }
        return Ok(());
    };
    let (pattern, placeholders) = regex_pattern(expected, &parts);
    let matcher = RegexBuilder::new(&pattern)
        .dot_matches_new_line(true)
        .build()?;
    let Some(captures) = matcher.captures(actual) else {
        mismatch(
            mismatches,
            path.to_owned(),
            format!("{expected:?}"),
            format!("{actual:?}"),
        );
        return Ok(());
    };
    for (index, placeholder) in placeholders.iter().enumerate() {
        let value = captures
            .name(&format!("placeholder_{index}"))
            .expect("capture exists")
            .as_str();
        match placeholder.as_str() {
            "timestamp" if chrono::DateTime::parse_from_rfc3339(value).is_err() => mismatch(
                mismatches,
                path.to_owned(),
                "{{timestamp}}".to_owned(),
                format!("{value:?}"),
            ),
            "http_date" if httpdate::parse_http_date(value).is_err() => mismatch(
                mismatches,
                path.to_owned(),
                "{{http_date}}".to_owned(),
                format!("{value:?}"),
            ),
            "uuid" if Uuid::parse_str(value).is_err() => mismatch(
                mismatches,
                path.to_owned(),
                "{{uuid}}".to_owned(),
                format!("{value:?}"),
            ),
            "timestamp" | "http_date" | "uuid" => {}
            name => match bindings.get(name) {
                Some(bound) if bound != value => mismatch(
                    mismatches,
                    path.to_owned(),
                    format!("{{{{{name}}}}}={bound:?}"),
                    format!("{value:?}"),
                ),
                Some(_) => {}
                None => {
                    bindings.insert(name.to_owned(), value.to_owned());
                }
            },
        }
    }
    Ok(())
}

fn pattern_parts(expected: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut remaining = expected;
    while let Some(start) = remaining.find("{{") {
        let end = remaining[start + 2..].find("}}")? + start + 2;
        if start > 0 {
            parts.push(remaining[..start].to_owned());
        }
        parts.push(remaining[start + 2..end].to_owned());
        remaining = &remaining[end + 2..];
    }
    if parts.is_empty() {
        return None;
    }
    if !remaining.is_empty() {
        parts.push(remaining.to_owned());
    }
    Some(parts)
}

fn regex_pattern(expected: &str, parts: &[String]) -> (String, Vec<String>) {
    let mut pattern = String::from("^");
    let mut placeholders = Vec::new();
    let mut remaining = expected;
    for part in parts {
        if let Some(literal) = remaining.strip_prefix(part) {
            pattern.push_str(&regex::escape(part));
            remaining = literal;
            continue;
        }
        let index = placeholders.len();
        pattern.push_str(&format!("(?P<placeholder_{index}>.*?)"));
        placeholders.push(part.clone());
        remaining = remaining
            .strip_prefix(&format!("{{{{{part}}}}}"))
            .expect("placeholder remains");
    }
    pattern.push_str(&regex::escape(remaining));
    pattern.push_str(r"\z");
    (pattern, placeholders)
}

pub(crate) fn render_diff(expected: &ResponseFixture, actual: &ResponseFixture) -> String {
    let expected = serde_json::to_string_pretty(expected).expect("response serializes");
    let actual = serde_json::to_string_pretty(actual).expect("response serializes");
    TextDiff::from_lines(&expected, &actual)
        .unified_diff()
        .header("expected", "actual")
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use rstest::rstest;
    use serde_json::json;

    use super::{render_diff, response_mismatches};
    use crate::model::ResponseFixture;

    fn response(status: u16, body: serde_json::Value) -> ResponseFixture {
        ResponseFixture {
            status,
            headers: BTreeMap::new(),
            body,
        }
    }

    #[rstest]
    #[case::binds_unbound_name(json!({"id": "{{dataset_id}}"}), json!({"id": "abc"}), None, Some("abc"), true)]
    #[case::matches_bound_name(json!({"id": "{{dataset_id}}"}), json!({"id": "abc"}), Some("abc"), Some("abc"), true)]
    #[case::rejects_different_bound_name(json!({"id": "{{dataset_id}}"}), json!({"id": "wrong"}), Some("abc"), Some("abc"), false)]
    #[case::matches_substring(json!({"name": "dataset-{{dataset_id}}-r1"}), json!({"name": "dataset-abc-r1"}), None, Some("abc"), true)]
    #[case::matches_placeholder_at_start(json!({"name": "{{dataset_id}}-suffix"}), json!({"name": "abc-suffix"}), None, Some("abc"), true)]
    #[case::matches_placeholder_at_end(json!({"name": "prefix-{{dataset_id}}"}), json!({"name": "prefix-abc"}), None, Some("abc"), true)]
    #[case::matches_repeated_name(json!({"name": "{{dataset_id}}/{{dataset_id}}"}), json!({"name": "abc/abc"}), None, Some("abc"), true)]
    #[case::rejects_repeated_name_mismatch(json!({"name": "{{dataset_id}}/{{dataset_id}}"}), json!({"name": "abc/other"}), None, Some("abc"), false)]
    #[case::matches_multiple_names(json!({"name": "{{dataset_id}}/r{{revision}}"}), json!({"name": "abc/r1"}), None, Some("abc"), true)]
    #[case::accepts_timestamp(json!({"at": "{{timestamp}}"}), json!({"at": "2025-01-02T03:04:05Z"}), None, None, true)]
    #[case::rejects_timestamp(json!({"at": "{{timestamp}}"}), json!({"at": "not-a-time"}), None, None, false)]
    #[case::accepts_http_date(json!({"at": "{{http_date}}"}), json!({"at": "Thu, 02 Jan 2025 03:04:05 GMT"}), None, None, true)]
    #[case::rejects_http_date(json!({"at": "{{http_date}}"}), json!({"at": "not-a-date"}), None, None, false)]
    #[case::accepts_uuid(json!({"id": "{{uuid}}"}), json!({"id": "f47ac10b-58cc-4372-a567-0e02b2c3d479"}), None, None, true)]
    #[case::rejects_uuid(json!({"id": "{{uuid}}"}), json!({"id": "not-a-uuid"}), None, None, false)]
    fn unifies_string_patterns(
        #[case] expected: serde_json::Value,
        #[case] actual: serde_json::Value,
        #[case] initial_binding: Option<&str>,
        #[case] expected_binding: Option<&str>,
        #[case] accepted: bool,
    ) {
        let bindings = initial_binding
            .map(|value| BTreeMap::from([("dataset_id".to_owned(), value.to_owned())]))
            .unwrap_or_default();
        let mut bindings = bindings;

        let mismatches = response_mismatches(
            &response(200, expected),
            &response(200, actual),
            &mut bindings,
        )
        .unwrap();

        if let Some(expected_binding) = expected_binding {
            assert_eq!(
                bindings.get("dataset_id").map(String::as_str),
                Some(expected_binding)
            );
        }
        assert_eq!(mismatches.is_empty(), accepted);
    }

    #[rstest]
    #[case::object_key_set(json!({"a": 1}), json!({"b": 1}), "$.body.a")]
    #[case::extra_object_key(json!({}), json!({"extra": 1}), "$.body.extra")]
    #[case::array_length(json!([1, 2]), json!([1]), "$.body.length")]
    #[case::array_order(json!([1, 2]), json!([2, 1]), "$.body[0]")]
    fn reports_structural_mismatches(
        #[case] expected_body: serde_json::Value,
        #[case] actual_body: serde_json::Value,
        #[case] path: &str,
    ) {
        let mismatches = response_mismatches(
            &response(200, expected_body),
            &response(200, actual_body),
            &mut BTreeMap::new(),
        )
        .unwrap();

        assert!(mismatches.iter().any(|mismatch| mismatch.path == path));
    }

    #[rstest]
    #[case::numeric_values(json!({"value": 1}), json!({"value": 2}), "$.body.value", "1", "2")]
    fn reports_exact_mismatch_values(
        #[case] expected_body: serde_json::Value,
        #[case] actual_body: serde_json::Value,
        #[case] path: &str,
        #[case] expected_value: &str,
        #[case] actual_value: &str,
    ) {
        let mismatches = response_mismatches(
            &response(200, expected_body),
            &response(200, actual_body),
            &mut BTreeMap::new(),
        )
        .unwrap();
        let mismatch = mismatches
            .iter()
            .find(|mismatch| mismatch.path == path)
            .unwrap();

        assert_eq!(mismatch.expected, expected_value);
        assert_eq!(mismatch.actual, actual_value);
    }

    #[rstest]
    #[case::status(200, 201, "$.status")]
    #[case::header(200, 200, "$.headers[\"content-type\"]")]
    fn reports_status_and_header_mismatches(
        #[case] expected_status: u16,
        #[case] actual_status: u16,
        #[case] path: &str,
    ) {
        let mut expected = response(expected_status, json!({}));
        let mut actual = response(actual_status, json!({}));
        expected
            .headers
            .insert("content-type".to_owned(), json!("application/json"));
        actual
            .headers
            .insert("content-type".to_owned(), json!("text/plain"));

        let mismatches = response_mismatches(&expected, &actual, &mut BTreeMap::new()).unwrap();

        assert!(mismatches.iter().any(|mismatch| mismatch.path == path));
    }

    #[rstest]
    #[case::body_change("before", "after")]
    fn renders_unified_response_diff(#[case] expected: &str, #[case] actual: &str) {
        let diff = render_diff(
            &response(200, json!({"value": expected})),
            &response(200, json!({"value": actual})),
        );

        assert!(diff.contains(&format!("-    \"value\": \"{expected}\"")));
        assert!(diff.contains(&format!("+    \"value\": \"{actual}\"")));
    }
}
