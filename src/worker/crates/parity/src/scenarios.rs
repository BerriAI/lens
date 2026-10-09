use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::runner::{Capture, CaptureSource, Scenario};

fn step(
    directory: &str,
    name: &str,
    method: &str,
    path: &str,
    headers: BTreeMap<String, String>,
    body: Value,
    captures: Vec<Capture>,
) -> Scenario {
    Scenario {
        directory: directory.to_owned(),
        name: name.to_owned(),
        request: crate::RequestFixture {
            method: method.to_owned(),
            path: path.to_owned(),
            headers,
            body,
        },
        captures,
    }
}

fn json_headers(authorization: Option<&str>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]);
    if let Some(value) = authorization {
        headers.insert("authorization".to_owned(), value.to_owned());
    }
    headers
}

fn bearer(token: &str) -> BTreeMap<String, String> {
    json_headers(Some(&format!("Bearer {token}")))
}

fn dataset_admin() -> BTreeMap<String, String> {
    bearer("{{admin_token}}")
}

fn cookie_session() -> BTreeMap<String, String> {
    BTreeMap::from([(
        "cookie".to_owned(),
        "lens_session={{session_cookie}}".to_owned(),
    )])
}

fn token_body(value: &str) -> Value {
    json!({"token": value})
}

fn session_cookie_capture() -> Vec<Capture> {
    vec![Capture {
        binding: "session_cookie".to_owned(),
        source: CaptureSource::SessionCookie,
    }]
}

fn dataset_capture() -> Vec<Capture> {
    vec![Capture {
        binding: "dataset_id".to_owned(),
        source: CaptureSource::JsonPointer("/id".to_owned()),
    }]
}

pub fn auth_scenarios() -> Vec<Scenario> {
    vec![
        step(
            "auth",
            "00_session_wrong_token",
            "POST",
            "/auth/session",
            json_headers(None),
            token_body("wrong-token"),
            vec![],
        ),
        step(
            "auth",
            "01_session_admin_token",
            "POST",
            "/auth/session",
            json_headers(None),
            token_body("{{admin_token}}"),
            session_cookie_capture(),
        ),
        step(
            "auth",
            "02_session_with_cookie",
            "GET",
            "/auth/session",
            cookie_session(),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "03_session_with_admin_bearer",
            "GET",
            "/auth/session",
            bearer("{{admin_token}}"),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "04_session_with_valid_jwt",
            "GET",
            "/auth/session",
            bearer("{{jwt.valid}}"),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "05_session_with_expired_jwt",
            "GET",
            "/auth/session",
            bearer("{{jwt.expired}}"),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "06_session_with_wrong_audience_jwt",
            "GET",
            "/auth/session",
            bearer("{{jwt.wrong_audience}}"),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "07_session_with_long_lifetime_jwt",
            "GET",
            "/auth/session",
            bearer("{{jwt.long_lifetime}}"),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "08_session_with_subject_mismatch_jwt",
            "GET",
            "/auth/session",
            bearer("{{jwt.sub_mismatch}}"),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "09_session_with_wrong_issuer_jwt",
            "GET",
            "/auth/session",
            bearer("{{jwt.wrong_issuer}}"),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "10_session_with_missing_iat_jwt",
            "GET",
            "/auth/session",
            bearer("{{jwt.missing_claim}}"),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "11_session_with_basic_auth",
            "GET",
            "/auth/session",
            BTreeMap::from([("authorization".to_owned(), "Basic abc".to_owned())]),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "12_session_without_credentials",
            "GET",
            "/auth/session",
            BTreeMap::new(),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "13_sign_out_with_evil_origin",
            "DELETE",
            "/auth/session",
            BTreeMap::from([
                (
                    "cookie".to_owned(),
                    "lens_session={{session_cookie}}".to_owned(),
                ),
                ("origin".to_owned(), "https://evil.example".to_owned()),
            ]),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "14_sign_out_without_origin",
            "DELETE",
            "/auth/session",
            cookie_session(),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "15_sign_out_with_lens_origin",
            "DELETE",
            "/auth/session",
            BTreeMap::from([
                (
                    "cookie".to_owned(),
                    "lens_session={{session_cookie}}".to_owned(),
                ),
                ("origin".to_owned(), "{{origin}}".to_owned()),
            ]),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "16_session_cookie_after_sign_out",
            "GET",
            "/auth/session",
            cookie_session(),
            Value::Null,
            vec![],
        ),
        step(
            "auth",
            "17_session_with_expired_database_cookie",
            "GET",
            "/auth/session",
            BTreeMap::from([(
                "cookie".to_owned(),
                "lens_session=parity-expired-session".to_owned(),
            )]),
            Value::Null,
            vec![],
        ),
    ]
}

pub fn dataset_scenarios() -> Vec<Scenario> {
    let admin = dataset_admin();
    let one_case = json!({
        "id": "provided-one",
        "messages": [{"role": "user", "content": "first case"}],
        "expected": "first expected",
        "included": true,
        "source": {}
    });
    let excluded_case = json!({
        "id": "provided-two",
        "messages": [{"role": "user", "content": "excluded case"}],
        "expected": "second expected",
        "included": false,
        "source": {}
    });
    let json_lines = concat!(
        "{\"messages\":[{\"role\":\"user\",\"content\":\"line one\"}],\"reply\":\"answer one\",\"expected\":\"expectation one\"}\n",
        "{\"messages\":[{\"role\":\"user\",\"content\":\"line two\"}],\"reply\":\"answer two\",\"expected\":\"expectation two\"}"
    );
    vec![
        step(
            "datasets",
            "00_list_empty",
            "GET",
            "/lens/datasets",
            admin.clone(),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "01_create_with_delegated_admin",
            "POST",
            "/lens/datasets",
            bearer("{{jwt.valid}}"),
            json!({"name": "parity dataset", "agent_name": "parity agent"}),
            dataset_capture(),
        ),
        step(
            "datasets",
            "02_list_after_create",
            "GET",
            "/lens/datasets",
            admin.clone(),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "03_read_latest",
            "GET",
            "/lens/datasets/{{dataset_id}}",
            admin.clone(),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "04_read_revision_zero",
            "GET",
            "/lens/datasets/{{dataset_id}}?revision=0",
            admin.clone(),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "05_build_from_plain_text",
            "POST",
            "/lens/datasets/build",
            admin.clone(),
            json!({
                "sources": [{"kind": "text", "text": "plain text dataset case"}],
                "dataset_id": "{{dataset_id}}"
            }),
            vec![],
        ),
        step(
            "datasets",
            "06_build_from_json_lines",
            "POST",
            "/lens/datasets/build",
            admin.clone(),
            json!({
                "sources": [{"kind": "text", "text": json_lines}],
                "dataset_id": "{{dataset_id}}"
            }),
            vec![],
        ),
        step(
            "datasets",
            "07_save_revision_with_included_and_excluded_cases",
            "POST",
            "/lens/datasets/{{dataset_id}}/revisions",
            admin.clone(),
            json!({"base_revision": 0, "cases": [one_case, excluded_case]}),
            vec![],
        ),
        step(
            "datasets",
            "08_save_revision_with_stale_base",
            "POST",
            "/lens/datasets/{{dataset_id}}/revisions",
            admin.clone(),
            json!({"base_revision": 0, "cases": [one_case, excluded_case]}),
            vec![],
        ),
        step(
            "datasets",
            "09_read_latest_after_revision",
            "GET",
            "/lens/datasets/{{dataset_id}}",
            admin.clone(),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "10_read_revision_zero_after_revision",
            "GET",
            "/lens/datasets/{{dataset_id}}?revision=0",
            admin.clone(),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "11_read_missing_revision",
            "GET",
            "/lens/datasets/{{dataset_id}}?revision=7",
            admin.clone(),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "12_read_missing_dataset",
            "GET",
            "/lens/datasets/does-not-exist",
            admin.clone(),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "13_export_latest",
            "GET",
            "/lens/datasets/{{dataset_id}}/export",
            admin.clone(),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "14_export_revision_zero",
            "GET",
            "/lens/datasets/{{dataset_id}}/export?revision=0",
            admin.clone(),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "15_read_included_revision_cases",
            "GET",
            "/lens/datasets/{{dataset_id}}/revisions/1/cases",
            admin.clone(),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "16_nonadmin_other_team_list",
            "GET",
            "/lens/datasets",
            bearer("{{jwt.other_team}}"),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "17_nonadmin_other_team_dataset",
            "GET",
            "/lens/datasets/{{dataset_id}}",
            bearer("{{jwt.other_team}}"),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "18_nonadmin_viewer_dataset",
            "GET",
            "/lens/datasets/{{dataset_id}}",
            bearer("{{jwt.other_team_viewer}}"),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "19_nonadmin_viewer_create",
            "POST",
            "/lens/datasets",
            bearer("{{jwt.other_team_viewer}}"),
            json!({"name": "forbidden dataset", "agent_name": "parity agent"}),
            vec![],
        ),
        step(
            "datasets",
            "20_nonadmin_viewer_list",
            "GET",
            "/lens/datasets",
            bearer("{{jwt.other_team_viewer}}"),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "21_nonadmin_viewer_revision_write",
            "POST",
            "/lens/datasets/{{dataset_id}}/revisions",
            bearer("{{jwt.other_team_viewer}}"),
            json!({"base_revision": 1, "cases": [one_case, excluded_case]}),
            vec![],
        ),
        step(
            "datasets",
            "22_nonadmin_viewer_build",
            "POST",
            "/lens/datasets/build",
            bearer("{{jwt.other_team_viewer}}"),
            json!({
                "sources": [{"kind": "text", "text": "viewer build dataset case"}],
                "dataset_id": "{{dataset_id}}"
            }),
            vec![],
        ),
        step(
            "datasets",
            "23_nonadmin_no_team_list",
            "GET",
            "/lens/datasets",
            bearer("{{jwt.no_team}}"),
            Value::Null,
            vec![],
        ),
        step(
            "datasets",
            "24_nonadmin_no_team_dataset",
            "GET",
            "/lens/datasets/{{dataset_id}}",
            bearer("{{jwt.no_team}}"),
            Value::Null,
            vec![],
        ),
    ]
}
