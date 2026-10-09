use lens_contract::ingestion::{
    IngestionCredential, IngestionKey, IngestionKeyCreated, IngestionKeyRequest, IngestionSnapshot,
};
use rstest::rstest;
use serde_json::json;

#[rstest]
fn empty_request_preserves_existing_defaults() {
    let request: IngestionKeyRequest = serde_json::from_value(json!({})).unwrap();
    assert_eq!(request, IngestionKeyRequest::default());
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        json!({
            "name": "Agent tracing", "team_id": "", "expires_at": null
        })
    );
}

#[rstest]
#[case::naive(json!({"expires_at": "2026-03-01T12:00:00"}))]
#[case::extra(json!({"token_hash": "injected"}))]
#[case::null_name(json!({"name": null}))]
#[case::numeric_team(json!({"team_id": 1}))]
fn request_rejects_untyped_or_unscoped_values(#[case] value: serde_json::Value) {
    assert!(serde_json::from_value::<IngestionKeyRequest>(value).is_err());
}

#[rstest]
fn stored_key_and_credential_require_explicit_nullable_expiry() {
    let tenant = json!({"user_id":"owner", "api_key_hash":"digest"});
    assert!(
        serde_json::from_value::<IngestionKey>(json!({
            "id":"key", "name":"Agent", "created_at":"2026-03-01T12:00:00Z", "tenant":tenant
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<IngestionCredential>(json!({
            "token_hash":"digest", "tenant":tenant
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<IngestionCredential>(json!({
            "token_hash":"digest", "tenant":tenant, "expires_at":null
        }))
        .is_ok()
    );
}

#[rstest]
fn creation_and_snapshot_preserve_python_wire_fields() {
    let value = json!({
        "key": "returned-only-on-create",
        "record": {
            "id": "key-id", "name": "Agent", "created_at": "2026-03-01T12:00:00.123456Z",
            "tenant": {"user_id": "owner", "api_key_hash": "digest"},
            "expires_at": null
        }
    });
    let created: IngestionKeyCreated = serde_json::from_value(value).unwrap();
    assert!(!created.active);
    assert_eq!(created.record.tenant.team_id, "");
    assert_eq!(created.record.tenant.org_id, "");
    assert_eq!(
        serde_json::to_value(created).unwrap(),
        json!({
            "key": "returned-only-on-create", "active": false,
            "record": {
                "id": "key-id", "name": "Agent", "created_at": "2026-03-01T12:00:00.123456Z",
                "tenant": {"team_id": "", "user_id": "owner", "org_id": "", "api_key_hash": "digest"},
                "expires_at": null
            }
        })
    );
    let snapshot: IngestionSnapshot =
        serde_json::from_value(json!({"issued_at": 1, "keys": []})).unwrap();
    assert_eq!(
        serde_json::to_value(snapshot).unwrap(),
        json!({"issued_at": 1, "keys": []})
    );
}
