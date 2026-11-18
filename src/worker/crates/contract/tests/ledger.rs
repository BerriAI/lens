use lens_contract::ledger::{
    LedgerBatch, LedgerEnvelope, LedgerRejection, LedgerResult, LedgerStatus, PayloadUpload,
};
use rstest::rstest;
use serde_json::json;

#[rstest]
fn envelope_reads_the_version_of_any_future_batch() {
    let envelope: LedgerEnvelope =
        serde_json::from_value(json!({"version": 2, "events": [], "added_later": true})).unwrap();
    assert_eq!(envelope.version, 2);
}

#[rstest]
#[case::unknown_batch_field(json!({"version": 1, "events": [], "extra": 1}))]
#[case::unknown_event_field(json!({"version": 1, "events": [{"event_id": "e", "record": {}, "extra": 1}]}))]
#[case::missing_record(json!({"version": 1, "events": [{"event_id": "e"}]}))]
fn version_one_batches_reject_unknown_shapes(#[case] body: serde_json::Value) {
    assert!(serde_json::from_value::<LedgerBatch>(body).is_err());
}

#[rstest]
fn rejection_reason_is_omitted_for_stored_events() {
    let stored = LedgerResult {
        event_id: "e".into(),
        status: LedgerStatus::Stored,
        reason: None,
    };
    let rejected = LedgerResult {
        event_id: "e".into(),
        status: LedgerStatus::Rejected,
        reason: Some(LedgerRejection::MissingEventTime),
    };
    assert_eq!(
        serde_json::to_value(stored).unwrap(),
        json!({"event_id": "e", "status": "stored"})
    );
    assert_eq!(
        serde_json::to_value(rejected).unwrap(),
        json!({"event_id": "e", "status": "rejected", "reason": "missing_event_time"})
    );
}

#[rstest]
fn payload_upload_requires_every_scope_field() {
    let body = json!({
        "version": 1, "event_id": "e", "field": "messages", "team_id": "t",
        "event_time_ms": 1, "bytes": 2, "sha256": "0".repeat(64), "content": "[]"
    });
    assert!(serde_json::from_value::<PayloadUpload>(body).is_err());
}
