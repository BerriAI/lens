mod support;

use lens_migrate::{Error, LegacySnapshot};
use rstest::rstest;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use support::{plan, source};

#[rstest]
fn populated_plan_preserves_authority_hashes_revisions_and_scheduling(source: Value) {
    let result = plan(source.clone()).unwrap();
    let records = result.records();
    assert_eq!(result.report().records, 11);
    assert_eq!(
        records["lens/%5B%22lens%22%5D"]["lens"],
        source["lenses"][0]["data"]
    );
    assert_eq!(
        records["lens/%5B%22lens%22%5D"]["due_at"],
        "2026-01-15T01:02:03.123456Z"
    );
    assert_eq!(
        records["worker-token/%5B%22keep-hash-byte-exact%22%5D"],
        source["workers"][0]["id"]
    );
    assert_eq!(
        records["ingestion-keys/catalog"][0],
        source["ingestion_keys"][0]["data"]
    );
    assert_eq!(
        records["dataset/%5B%22dataset%22%2C1%5D"],
        source["datasets"][1]["data"]
    );
    assert_eq!(
        records["dataset-latest/%5B%22dataset%22%5D"]["summary"]["revision"],
        2
    );
    assert_eq!(
        records["dataset-latest/%5B%22dataset%22%5D"]["summary"]["updated_at"],
        "2026-01-17T00:00:00Z"
    );
    assert_eq!(
        records["trace-signal/%5B%22trace%22%2C%22backend-key%22%5D"]["config_key"],
        "keep-config-key"
    );
    assert_eq!(
        records["trace-signal/%5B%22trace%22%2C%22backend-key%22%5D"]["data"],
        source["trace_signals"][0]["data"]
    );
}

#[rstest]
fn archived_jobs_keep_full_body_and_zero_version(source: Value) {
    let result = plan(source.clone()).unwrap();
    let entry = result
        .records()
        .values()
        .find(|value| value.get("archived_version").is_some())
        .unwrap();
    assert_eq!(entry["archived_version"], 0);
    assert_eq!(entry["parent_key"], "lens/%5B%22lens%22%5D");
    assert_eq!(entry["job"], source["runs"][0]["data"]);
}

#[rstest]
fn arbitrary_legacy_row_order_does_not_change_fingerprint(mut source: Value) {
    let original = plan(source.clone()).unwrap();
    source["datasets"].as_array_mut().unwrap().reverse();
    assert_eq!(original.report(), plan(source).unwrap().report());
}

#[rstest]
fn source_change_changes_fingerprint(mut source: Value) {
    let original = plan(source.clone()).unwrap();
    source["lenses"][0]["data"]["settings"]["context"] = json!("Changed criteria");
    assert_ne!(
        original.report().fingerprint,
        plan(source).unwrap().report().fingerprint
    );
}

#[rstest]
fn null_scheduling_is_preserved_without_recomputation(mut source: Value) {
    source["lenses"][0]["due_at"] = Value::Null;
    assert!(plan(source).unwrap().records()["lens/%5B%22lens%22%5D"]["due_at"].is_null());
}

#[rstest]
#[case::initial_row_version(0, 0, 1, true)]
#[case::advanced_versions(42, 3, 2, true)]
#[case::zero_revision(0, 0, 0, true)]
#[case::negative_stored_version(-1,0,1,false)]
#[case::negative_public_version(42,-1,1,false)]
#[case::negative_revision(42,0,-1,false)]
fn lens_versions_are_independent_nonnegative_counters(
    mut source: Value,
    #[case] stored: i64,
    #[case] public: i64,
    #[case] revision: i64,
    #[case] valid: bool,
) {
    source["lenses"][0]["version"] = json!(stored);
    source["lenses"][0]["data"]["version"] = json!(public);
    source["lenses"][0]["data"]["revision"] = json!(revision);
    assert_eq!(plan(source).is_ok(), valid);
}

#[rstest]
#[case::negative(-1,false)]
#[case::initial(0, true)]
#[case::higher(3, true)]
fn dataset_identity_revision_is_nonnegative(
    mut source: Value,
    #[case] revision: i64,
    #[case] valid: bool,
) {
    source["datasets"][0]["revision"] = json!(revision);
    source["datasets"][0]["data"]["revision"] = json!(revision);
    assert_eq!(plan(source).is_ok(), valid);
}

#[rstest]
fn zero_span_signal_is_preserved(mut source: Value) {
    source["trace_signals"][0]["span_count"] = json!(0);
    let result = plan(source.clone()).unwrap();
    assert_eq!(
        result.records()["trace-signal/%5B%22trace%22%2C%22backend-key%22%5D"]["span_count"],
        0
    );
    assert_eq!(
        result.records()["trace-signal/%5B%22trace%22%2C%22backend-key%22%5D"]["data"],
        source["trace_signals"][0]["data"]
    );
}

#[rstest]
fn identifiers_use_python_compatible_utf8_percent_encoding(mut source: Value) {
    source["lenses"][0]["id"] = json!("a/雪 ._-~");
    source["lenses"][0]["data"]["id"] = json!("a/雪 ._-~");
    assert!(
        plan(source)
            .unwrap()
            .records()
            .contains_key("lens/%5B%22a%2F%E9%9B%AA%20._-~%22%5D")
    );
}

#[rstest]
#[case::lens("lenses","id",json!("wrong"))]
#[case::lens_version("lenses","version",json!(-1))]
#[case::run("runs","id",json!("wrong"))]
#[case::review("reviews","execution_id",json!("wrong"))]
#[case::worker("workers","id",json!("wrong"))]
#[case::worker_hash("workers","token_hash",json!(""))]
#[case::ingestion("ingestion_keys","id",json!("wrong"))]
#[case::dataset("datasets","id",json!("wrong"))]
#[case::revision("datasets","revision",json!(99))]
#[case::configuration("signal_configs","id",json!("other"))]
#[case::span_count("trace_signals","span_count",json!(-1))]
#[case::trace_data("trace_signals","data",json!([]))]
fn inconsistent_source_rows_are_rejected(
    mut source: Value,
    #[case] table: &str,
    #[case] field: &str,
    #[case] value: Value,
) {
    source[table][0][field] = value;
    assert!(matches!(plan(source), Err(Error::InvalidRecord)));
}

#[rstest]
#[case::empty(String::new())]
#[case::short("a".repeat(63))]
#[case::long("a".repeat(65))]
#[case::nonhex(format!("{}g", "a".repeat(63)))]
#[case::nonascii("é".repeat(32))]
fn invalid_ingestion_hashes_are_rejected(mut source: Value, #[case] hash: String) {
    source["ingestion_keys"][0]["data"]["tenant"]["api_key_hash"] = json!(hash);
    assert!(matches!(plan(source), Err(Error::InvalidRecord)));
}

#[rstest]
#[case::lowercase(false)]
#[case::uppercase(true)]
fn valid_ingestion_hashes_are_preserved(mut source: Value, #[case] uppercase: bool) {
    let hash = source["ingestion_keys"][0]["data"]["tenant"]["api_key_hash"]
        .as_str()
        .unwrap();
    let hash = if uppercase {
        hash.to_ascii_uppercase()
    } else {
        hash.to_owned()
    };
    source["ingestion_keys"][0]["data"]["tenant"]["api_key_hash"] = json!(hash);
    assert_eq!(
        plan(source.clone()).unwrap().records()["ingestion-keys/catalog"][0],
        source["ingestion_keys"][0]["data"]
    );
}

#[rstest]
#[case::same_hash(false)]
#[case::distinct_hash(true)]
fn ingestion_credentials_require_unique_hashes(mut source: Value, #[case] distinct: bool) {
    let mut second = source["ingestion_keys"][0].clone();
    second["id"] = json!("second-key");
    second["data"]["id"] = json!("second-key");
    second["data"]["tenant"]["team_id"] = json!("another-team");
    if distinct {
        second["data"]["tenant"]["api_key_hash"] = json!(format!(
            "{:x}",
            Sha256::digest(b"second tracing credential")
        ));
    }
    source["ingestion_keys"]
        .as_array_mut()
        .unwrap()
        .push(second);
    let result = plan(source.clone());
    if distinct {
        assert_eq!(
            result.unwrap().records()["ingestion-keys/catalog"],
            json!([
                source["ingestion_keys"][0]["data"],
                source["ingestion_keys"][1]["data"]
            ])
        );
    } else {
        assert!(matches!(result, Err(Error::Duplicate)));
    }
}

#[rstest]
#[case::lens("lenses")]
#[case::run("runs")]
#[case::review("reviews")]
#[case::worker("workers")]
#[case::ingestion("ingestion_keys")]
#[case::dataset("datasets")]
#[case::configuration("signal_configs")]
#[case::trace("trace_signals")]
fn duplicate_identities_never_replace_source_records(mut source: Value, #[case] table: &str) {
    let duplicate = source[table][0].clone();
    source[table].as_array_mut().unwrap().push(duplicate);
    assert!(matches!(plan(source), Err(Error::Duplicate)));
}

#[rstest]
#[case::lens("lenses")]
#[case::run("runs")]
#[case::review("reviews")]
#[case::worker("workers")]
#[case::ingestion("ingestion_keys")]
#[case::dataset("datasets")]
#[case::configuration("signal_configs")]
fn positional_arrays_are_never_treated_as_object_records(mut source: Value, #[case] table: &str) {
    source[table][0]["data"] = json!([]);
    assert!(matches!(plan(source), Err(Error::InvalidRecord)));
}

#[rstest]
fn empty_source_produces_no_synthetic_live_records() {
    let result = LegacySnapshot::default().plan().unwrap();
    assert!(result.records().is_empty());
    assert_eq!(result.report().records, 0);
}
