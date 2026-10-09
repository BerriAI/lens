use super::{Scenario, dataset_admin, step};
use serde_json::{Value, json};

fn request(name: &str, method: &str, path: &str, body: Value) -> Scenario {
    step("signals", name, method, path, dataset_admin(), body, vec![])
}

fn config(name: &str, body: Value) -> Scenario {
    request(name, "PUT", "/lens/signals", body)
}

pub fn signal_scenarios() -> Vec<Scenario> {
    vec![
        config("00_reset_default", json!({})),
        request("01_default", "GET", "/lens/signals", Value::Null),
        request(
            "02_unclassified",
            "POST",
            "/lens/traces/signals",
            json!({"traces":[{"trace_id":"missing"},{"trace_id":"missing","trace_ref":"ref"}]}),
        ),
        config("03_threshold_lower", json!({"threshold":0.04})),
        config("04_threshold_upper", json!({"threshold":0.96})),
        config("05_threshold_null", json!({"threshold":null})),
        config("06_unknown_field", json!({"enabled":true})),
        config(
            "07_invalid_id",
            json!({"signals":[{"id":"Bad-id","name":"Bad","question":"Question?"}]}),
        ),
        config(
            "08_short_name",
            json!({"signals":[{"id":"signal","name":"","question":"Question?"}]}),
        ),
        config(
            "09_long_name",
            json!({"signals":[{"id":"signal","name":"x".repeat(61),"question":"Question?"}]}),
        ),
        config(
            "10_short_question",
            json!({"signals":[{"id":"signal","name":"Signal","question":"x"}]}),
        ),
        config(
            "11_long_question",
            json!({"signals":[{"id":"signal","name":"Signal","question":"x".repeat(501)}]}),
        ),
        config(
            "12_duplicate_ids",
            json!({"signals":[{"id":"signal","name":"First","question":"Question?"},{"id":"signal","name":"Second","question":"Question?"}]}),
        ),
        config(
            "13_too_many",
            json!({"signals":(0..21).map(|id|json!({"id":format!("s{id}"),"name":"Signal","question":"Question?"})).collect::<Vec<_>>()}),
        ),
        config(
            "14_unconfigured_model",
            json!({"model":"unconfigured-evaluation-model"}),
        ),
        config(
            "15_custom_disabled",
            json!({"threshold":"0.95","signals":[{"id":"failed_lookup","name":"Lookup failed","question":"Did the lookup fail?"}]}),
        ),
        request("16_saved_config", "GET", "/lens/signals", Value::Null),
        request(
            "17_empty_traces",
            "POST",
            "/lens/traces/signals",
            json!({"traces":[]}),
        ),
        request(
            "18_invalid_trace",
            "POST",
            "/lens/traces/signals",
            json!({"traces":[{"trace_id":"","trace_ref":false}]}),
        ),
        config("19_empty_signals", json!({"signals":[],"threshold":0.05})),
        request("20_empty_config", "GET", "/lens/signals", Value::Null),
        config("21_restore_default", json!({})),
    ]
}
