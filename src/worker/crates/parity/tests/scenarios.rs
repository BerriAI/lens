use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::json;

use lens_parity::scenarios::{auth_scenarios, dataset_scenarios};
use lens_parity::{CaptureSource, Scenario};

#[rstest]
#[case::auth(auth_scenarios, 18)]
#[case::datasets(dataset_scenarios, 25)]
fn scenario_sets_cover_required_routes(
    #[case] scenarios: fn() -> Vec<Scenario>,
    #[case] expected_count: usize,
) {
    assert_eq!(scenarios().len(), expected_count);
}

#[rstest]
#[case::auth(())]
fn auth_steps_keep_json_headers_cookie_templates_and_captures(#[case] _case: ()) {
    let scenarios = auth_scenarios();

    assert_eq!(
        scenarios[0].request.headers,
        BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())])
    );
    assert_eq!(scenarios[0].request.body, json!({"token": "wrong-token"}));
    assert_eq!(
        scenarios[2].request.headers,
        BTreeMap::from([(
            "cookie".to_owned(),
            "lens_session={{session_cookie}}".to_owned()
        )])
    );
    assert_eq!(
        scenarios[4].request.headers,
        BTreeMap::from([
            (
                "authorization".to_owned(),
                "Bearer {{jwt.valid}}".to_owned()
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
        ])
    );
    assert_eq!(scenarios[1].captures.len(), 1);
    assert_eq!(scenarios[1].captures[0].binding, "session_cookie");
    assert!(matches!(
        &scenarios[1].captures[0].source,
        CaptureSource::SessionCookie
    ));
}

#[rstest]
#[case::datasets(())]
fn dataset_steps_keep_admin_headers_and_id_capture(#[case] _case: ()) {
    let scenarios = dataset_scenarios();

    assert_eq!(
        scenarios[0].request.headers,
        BTreeMap::from([
            (
                "authorization".to_owned(),
                "Bearer {{admin_token}}".to_owned()
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
        ])
    );
    assert_eq!(scenarios[1].captures.len(), 1);
    assert_eq!(scenarios[1].captures[0].binding, "dataset_id");
    match &scenarios[1].captures[0].source {
        CaptureSource::JsonPointer(pointer) => assert_eq!(pointer, "/id"),
        CaptureSource::SessionCookie => panic!("dataset capture must use a JSON pointer"),
    }
}
