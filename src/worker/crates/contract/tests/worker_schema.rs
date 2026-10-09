use rstest::rstest;
use serde_json::Value;

#[rstest]
fn generated_worker_schema_preserves_the_current_contract() {
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/python-worker-v7.json")).unwrap();
    compare("", &lens_contract::schema::worker_contract(), &expected);
}

#[rstest]
fn checked_in_worker_schema_matches_rust() {
    assert_eq!(
        format!(
            "{}\n",
            serde_json::to_string_pretty(&lens_contract::schema::worker_contract()).unwrap()
        ),
        include_str!("../../../../../schema/lens-worker.v7.json")
    );
}

fn compare(path: &str, actual: &Value, expected: &Value) {
    match (actual, expected) {
        (Value::Object(left), Value::Object(right)) => {
            if path != "/definitions" {
                assert_eq!(
                    left.keys().collect::<Vec<_>>(),
                    right.keys().collect::<Vec<_>>(),
                    "{path}"
                );
            }
            for (key, value) in right {
                compare(&format!("{path}/{key}"), &left[key], value);
            }
        }
        (Value::Array(left), Value::Array(right)) => {
            assert_eq!(left.len(), right.len(), "{path}");
            for (index, (left, right)) in left.iter().zip(right).enumerate() {
                compare(&format!("{path}/{index}"), left, right);
            }
        }
        _ => assert_eq!(actual, expected, "{path}"),
    }
}
