use std::collections::BTreeMap;
use std::sync::LazyLock;

use chrono::DateTime;
use regex::Regex;
use serde_json::Value;
use uuid::Uuid;

use crate::model::ResponseFixture;

static RFC3339: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})\b").unwrap()
});
static SQL_TIMESTAMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d+)?\b").unwrap());
static HTTP_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun), \d{2} (?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) \d{4} \d{2}:\d{2}:\d{2} GMT\b").unwrap()
});
static UUID_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b")
        .unwrap()
});

pub(crate) fn normalize_response(
    response: &mut ResponseFixture,
    bindings: &BTreeMap<String, String>,
) {
    response.body = normalize_value(response.body.clone(), bindings);
    response.headers = response
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), normalize_value(value.clone(), bindings)))
        .collect();
}

fn normalize_value(value: Value, bindings: &BTreeMap<String, String>) -> Value {
    match value {
        Value::String(text) => Value::String(normalize_text(&text, bindings)),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| normalize_value(value, bindings))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, normalize_value(value, bindings)))
                .collect(),
        ),
        scalar => scalar,
    }
}

fn normalize_text(text: &str, bindings: &BTreeMap<String, String>) -> String {
    let mut normalized = text.to_owned();
    let mut replacements: Vec<(&str, &str)> = bindings
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, value)| (value.as_str(), name.as_str()))
        .collect();
    replacements.sort_by_key(|(value, name)| (std::cmp::Reverse(value.len()), *name));
    for (value, name) in replacements {
        normalized = normalized.replace(value, &format!("{{{{{name}}}}}"));
    }
    normalized = RFC3339
        .replace_all(&normalized, |matched: &regex::Captures<'_>| {
            if DateTime::parse_from_rfc3339(&matched[0]).is_ok() {
                "{{timestamp}}".to_owned()
            } else {
                matched[0].to_owned()
            }
        })
        .into_owned();
    normalized = SQL_TIMESTAMP
        .replace_all(&normalized, |matched: &regex::Captures<'_>| {
            if chrono::NaiveDateTime::parse_from_str(&matched[0], "%Y-%m-%d %H:%M:%S%.f").is_ok() {
                "{{sql_timestamp}}".to_owned()
            } else {
                matched[0].to_owned()
            }
        })
        .into_owned();
    normalized = HTTP_DATE
        .replace_all(&normalized, |matched: &regex::Captures<'_>| {
            if httpdate::parse_http_date(&matched[0]).is_ok() {
                "{{http_date}}".to_owned()
            } else {
                matched[0].to_owned()
            }
        })
        .into_owned();
    UUID_PATTERN
        .replace_all(&normalized, |matched: &regex::Captures<'_>| {
            if Uuid::parse_str(&matched[0]).is_ok() {
                "{{uuid}}".to_owned()
            } else {
                matched[0].to_owned()
            }
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use rstest::rstest;
    use serde_json::{Value, json};

    use super::normalize_response;
    use crate::model::ResponseFixture;

    #[rstest]
    #[case::captured_values("dataset-abc-r1", "dataset-{{dataset_id}}-r1", "abc")]
    #[case::timestamp("2025-01-02T03:04:05Z", "{{timestamp}}", "")]
    #[case::sql_timestamp("2025-01-02 03:04:05.123456789", "{{sql_timestamp}}", "")]
    #[case::invalid_sql_timestamp("2025-99-02 03:04:05", "2025-99-02 03:04:05", "")]
    #[case::http_date("Thu, 02 Jan 2025 03:04:05 GMT", "{{http_date}}", "")]
    #[case::uuid("f47ac10b-58cc-4372-a567-0e02b2c3d479", "{{uuid}}", "")]
    fn normalizes_bound_and_volatile_strings(
        #[case] body: &str,
        #[case] expected: &str,
        #[case] bound: &str,
    ) {
        let mut response = ResponseFixture {
            status: 200,
            headers: BTreeMap::from([("content-disposition".to_owned(), json!(body))]),
            body: json!({"value": body}),
        };
        let bindings = if bound.is_empty() {
            BTreeMap::new()
        } else {
            BTreeMap::from([("dataset_id".to_owned(), bound.to_owned())])
        };

        normalize_response(&mut response, &bindings);

        assert_eq!(response.body, json!({"value": expected}));
        assert_eq!(
            response.headers.get("content-disposition"),
            Some(&json!(expected))
        );
    }

    #[rstest]
    #[case::replaces_multiple_occurrences("abc/abc", "{{dataset_id}}/{{dataset_id}}")]
    #[case::retains_unrecognized_text("plain", "plain")]
    fn normalizes_substrings_without_changing_other_text(
        #[case] body: &str,
        #[case] expected: &str,
    ) {
        let mut response = ResponseFixture {
            status: 200,
            headers: BTreeMap::new(),
            body: json!({"value": body}),
        };

        normalize_response(
            &mut response,
            &BTreeMap::from([("dataset_id".to_owned(), "abc".to_owned())]),
        );

        assert_eq!(response.body, json!({"value": expected}));
    }

    #[rstest]
    #[case::session_cookie(
        "lens_session=secret-value; expires=Thu, 02 Jan 2025 03:04:05 GMT; HttpOnly; SameSite=lax; Secure; Max-Age=60; Path=/",
        "lens_session={{session_cookie}}; expires={{http_date}}; HttpOnly; SameSite=lax; Secure; Max-Age=60; Path=/"
    )]
    fn normalizes_cookie_values_but_preserves_attributes(
        #[case] cookie: &str,
        #[case] expected: &str,
    ) {
        let mut response = ResponseFixture {
            status: 200,
            headers: BTreeMap::from([("set-cookie".to_owned(), json!(cookie))]),
            body: Value::Null,
        };

        normalize_response(
            &mut response,
            &BTreeMap::from([("session_cookie".to_owned(), "secret-value".to_owned())]),
        );

        assert_eq!(response.headers.get("set-cookie"), Some(&json!(expected)));
    }
}
