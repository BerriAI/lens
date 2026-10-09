use super::{Scenario, dataset_admin, json_headers, step};
use base64::{Engine, engine::general_purpose::URL_SAFE};
use serde_json::{Value, json};

fn request(name: &str, method: &str, path: &str, body: Value) -> Scenario {
    step(
        "activity",
        name,
        method,
        path,
        dataset_admin(),
        body,
        vec![],
    )
}

fn preview(name: &str, body: Value) -> Scenario {
    request(name, "POST", "/lens/preview/sample", body)
}

pub fn activity_scenarios() -> Vec<Scenario> {
    let id =
        URL_SAFE.encode("[\"traces\", \"alpha\", \"f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0\", \"\"]");
    let path = format!("/lens/parity-lens/executions/{id}");
    let foreign = URL_SAFE.encode("[\"traces\", \"beta\", \"trace\", \"\"]");
    let wrong = URL_SAFE.encode("[\"unknown\", \"alpha\", \"trace\", \"\"]");
    vec![
        step(
            "activity",
            "00_auth_required",
            "GET",
            "/lens/activity/available",
            json_headers(None),
            Value::Null,
            vec![],
        ),
        request(
            "01_availability",
            "GET",
            "/lens/activity/available",
            Value::Null,
        ),
        request("02_agents", "GET", "/lens/agents", Value::Null),
        preview("03_selection_required", json!({})),
        preview("04_selection_null", json!({"selection":null})),
        preview("05_source_invalid", json!({"selection":{"source":"all"}})),
        preview(
            "06_selection_extra",
            json!({"selection":{"all_teams":true}}),
        ),
        preview(
            "07_invalid_execution",
            json!({"selection":{"execution_ids":["invalid"]}}),
        ),
        preview(
            "08_invalid_execution_source",
            json!({"selection":{"execution_ids":[wrong]}}),
        ),
        preview("09_percent_zero", json!({"selection":{"sample_percent":0}})),
        preview(
            "10_percent_maximum",
            json!({"selection":{"sample_percent":101}}),
        ),
        preview(
            "11_sample_size_zero",
            json!({"selection":{"sample_size":0}}),
        ),
        preview("12_offset_negative", json!({"selection":{},"offset":-1})),
        preview("13_offset_fraction", json!({"selection":{},"offset":1.5})),
        preview(
            "14_lookback_zero",
            json!({"selection":{},"lookback_hours":0}),
        ),
        preview(
            "15_lookback_calendar",
            json!({"selection":{},"lookback_hours":1000000000000u64}),
        ),
        preview(
            "16_datetime_naive",
            json!({"selection":{},"as_of":"2026-01-01T00:00:00"}),
        ),
        preview(
            "17_datetime_invalid",
            json!({"selection":{},"as_of":"yesterday"}),
        ),
        preview(
            "18_empty_window",
            json!({"selection":{},"as_of":"2020-01-01T00:00:00Z"}),
        ),
        preview(
            "19_extra_fields_ignored",
            json!({"selection":{},"as_of":"2020-01-01T00:00:00Z","unused":true}),
        ),
        preview(
            "20_large_offset",
            json!({"selection":{},"offset":4294967296u64}),
        ),
        preview(
            "21_filter_invalid",
            json!({"selection":{"filters":[{"key":"","value":7}]}}),
        ),
        request(
            "22_lens_missing",
            "GET",
            &format!("/lens/missing/executions/{id}"),
            Value::Null,
        ),
        request(
            "23_execution_invalid",
            "GET",
            "/lens/parity-lens/executions/invalid",
            Value::Null,
        ),
        request(
            "24_execution_foreign_team",
            "GET",
            &format!("/lens/parity-lens/executions/{foreign}"),
            Value::Null,
        ),
        request("25_execution_content", "GET", &path, Value::Null),
        request(
            "26_execution_cursor",
            "GET",
            &format!("{path}?cursor=zzzz"),
            Value::Null,
        ),
        request(
            "27_execution_offset_negative",
            "GET",
            &format!("{path}?offset=-1"),
            Value::Null,
        ),
        request(
            "28_execution_offset_fraction",
            "GET",
            &format!("{path}?offset=1.5"),
            Value::Null,
        ),
        request(
            "29_execution_unknown_query",
            "GET",
            &format!("{path}?unused=true"),
            Value::Null,
        ),
        request(
            "30_findings_missing",
            "POST",
            "/lens/traces/findings",
            json!({"traces":[{"trace_id":"missing"},{"trace_id":"missing","trace_ref":"ref"}]}),
        ),
        request(
            "31_findings_empty",
            "POST",
            "/lens/traces/findings",
            json!({"traces":[]}),
        ),
        request(
            "32_findings_invalid",
            "POST",
            "/lens/traces/findings",
            json!({"traces":[{"trace_id":"","trace_ref":false}]}),
        ),
        request(
            "33_findings_unknown_field",
            "POST",
            "/lens/traces/findings",
            json!({"traces":[{"trace_id":"trace","scope":"all"}]}),
        ),
        preview(
            "34_unsigned_offset",
            json!({"selection":{},"offset":9223372036854775808u64}),
        ),
        preview(
            "35_maximum_offset",
            json!({"selection":{},"offset":u64::MAX}),
        ),
        preview(
            "36_offset_overflow",
            json!({"selection":{},"offset":"18446744073709551616"}),
        ),
        request(
            "37_content_offset_overflow",
            "GET",
            &format!("{path}?offset=4294967295"),
            Value::Null,
        ),
    ]
}
