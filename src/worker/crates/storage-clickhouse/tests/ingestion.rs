#[path = "ingestion/support.rs"]
mod support;

use chrono::{DateTime, Duration, Utc};
use lens_auth::{
    IngestionError, StoreError,
    ingestion::{INGESTION_KEY_LIMIT, Ingestion, IngestionRepository},
    local_admin,
};
use lens_contract::ingestion::IngestionKeyRequest;
use litellm_http::Client;
use litellm_storage_clickhouse::{Connection, ingestion::IngestionKeys, state::ClickHouseState};
use rstest::rstest;
use serde_json::json;
use support::{
    CATALOG, Database, catalog, concurrent_insert_and_revoke, concurrent_inserts, database,
    isolated_database, key, now, survivors,
};

#[rstest]
#[tokio::test]
async fn empty_catalog_can_revoke_missing_keys(#[future(awt)] database: Database) {
    let repository = database.repository();
    assert!(repository.list().await.unwrap().is_empty());
    repository.revoke("missing").await.unwrap();
    assert!(repository.list().await.unwrap().is_empty());
}

#[rstest]
#[tokio::test]
async fn independent_clients_read_hash_only_records_in_id_order(
    #[future(awt)] database: Database,
    now: DateTime<Utc>,
) {
    let service = Ingestion(database.repository());
    let created = service
        .create(&local_admin(), IngestionKeyRequest::default(), now)
        .await
        .unwrap();
    let first = key("000-first");
    database.repository().insert(&first).await.unwrap();
    assert_eq!(
        database.repository().list().await.unwrap(),
        vec![first, created.record]
    );
    let stored = database.store.read(CATALOG).await.unwrap();
    assert!(!stored.value.to_string().contains(&created.key));
    assert_eq!(stored.value.as_array().unwrap().len(), 2);
}

#[rstest]
#[tokio::test]
async fn duplicate_id_cannot_replace_key_or_owner(#[future(awt)] database: Database) {
    let original = key("one");
    database.repository().insert(&original).await.unwrap();
    let mut replacement = original.clone();
    replacement.tenant.user_id = "other-owner".into();
    replacement.tenant.api_key_hash = "b".repeat(64);
    assert!(matches!(
        database.repository().insert(&replacement).await,
        Err(IngestionError::AlreadyExists)
    ));
    assert_eq!(database.repository().list().await.unwrap(), vec![original]);
}

#[rstest]
#[tokio::test]
async fn concurrent_duplicate_ids_have_one_winner(#[future(awt)] database: Database) {
    let results = concurrent_inserts(&database, true).await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(IngestionError::AlreadyExists)))
            .count(),
        15
    );
    assert_eq!(
        database.repository().list().await.unwrap(),
        vec![key("one-id")]
    );
}

#[rstest]
#[tokio::test]
async fn concurrent_distinct_creations_retain_every_key(#[future(awt)] database: Database) {
    let results = concurrent_inserts(&database, false).await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 16);
    assert_eq!(database.repository().list().await.unwrap().len(), 16);
}

#[rstest]
#[tokio::test]
async fn concurrent_creations_and_revocations_never_restore_removed_keys(
    #[future(awt)] database: Database,
) {
    database.seed(catalog(16)).await;
    concurrent_insert_and_revoke(&database).await;
    assert_eq!(database.repository().list().await.unwrap(), survivors());
}

#[rstest]
#[tokio::test]
async fn full_catalog_requires_revocation_before_another_creation(
    #[future(awt)] database: Database,
) {
    database.seed(catalog(INGESTION_KEY_LIMIT - 1)).await;
    let repository = database.repository();
    let last = key("last");
    repository.insert(&last).await.unwrap();
    assert_eq!(repository.list().await.unwrap().len(), INGESTION_KEY_LIMIT);
    assert!(matches!(
        repository.insert(&key("overflow")).await,
        Err(IngestionError::KeyLimit)
    ));
    repository.revoke(&last.id).await.unwrap();
    repository.insert(&key("replacement")).await.unwrap();
    assert_eq!(repository.list().await.unwrap().len(), INGESTION_KEY_LIMIT);
}

#[rstest]
#[tokio::test]
async fn oversized_catalog_fails_closed_but_revocation_can_repair_it(
    #[future(awt)] database: Database,
) {
    database.seed(catalog(INGESTION_KEY_LIMIT + 1)).await;
    let repository = database.repository();
    assert!(matches!(
        repository.list().await,
        Err(IngestionError::CatalogLimit)
    ));
    repository.revoke("00000000").await.unwrap();
    assert_eq!(repository.list().await.unwrap().len(), INGESTION_KEY_LIMIT);
}

#[rstest]
#[case::object(json!({"unexpected": true}))]
#[case::malformed_record(json!([{"id": "broken"}]))]
#[case::missing_expiry(json!([{"id": "key", "name": "Agent", "created_at": "2026-03-01T12:00:00Z", "tenant": {"user_id": "owner", "api_key_hash": "a".repeat(64)}}]))]
#[case::unknown_fields(json!([{"id": "key", "name": "Agent", "created_at": "2026-03-01T12:00:00Z", "tenant": {"user_id": "owner", "api_key_hash": "digest"}, "expires_at": null, "key": "raw-secret"}]))]
#[tokio::test]
async fn malformed_catalog_is_not_silently_replaced(
    #[future(awt)] database: Database,
    #[case] value: serde_json::Value,
) {
    database.seed(value.clone()).await;
    let repository = database.repository();
    assert!(matches!(
        repository.list().await,
        Err(IngestionError::Store(StoreError::Unavailable(_)))
    ));
    assert!(matches!(
        repository.insert(&key("new")).await,
        Err(IngestionError::Store(StoreError::Unavailable(_)))
    ));
    assert!(matches!(
        repository.revoke("key").await,
        Err(IngestionError::Store(StoreError::Unavailable(_)))
    ));
    assert_eq!(database.store.read(CATALOG).await.unwrap().value, value);
}

#[rstest]
#[tokio::test]
async fn legacy_python_records_preserve_precision_and_filter_negative_expiry(
    #[future(awt)] database: Database,
    now: DateTime<Utc>,
) {
    database.seed(json!([
        {"id": "expired", "name": "Old", "created_at": "2026-03-01T07:00:00.123456-05:00", "tenant": {"user_id": "owner", "api_key_hash": "a".repeat(64)}, "expires_at": -1},
        {"id": "current", "name": "Current", "created_at": "2026-03-01T12:00:00.123456Z", "tenant": {"team_id": "legacy-team", "user_id": "legacy-owner", "org_id": "legacy-org", "api_key_hash": "b".repeat(64)}, "expires_at": null}
    ])).await;
    let service = Ingestion(database.repository());
    let records = service.list(&local_admin()).await.unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].created_at, now);
    assert_eq!(records[1].expires_at, Some(-1));
    let snapshot = service.current_snapshot(now).await.unwrap();
    assert_eq!(snapshot.keys.len(), 1);
    assert_eq!(snapshot.keys[0].tenant, records[0].tenant);
    assert_eq!(snapshot.keys[0].token_hash, "b".repeat(64));
}

#[rstest]
#[tokio::test]
async fn restart_preserves_creation_expiry_and_revocation(
    #[future(awt)] isolated_database: Database,
    now: DateTime<Utc>,
) {
    let service = Ingestion(isolated_database.repository());
    let keep = service
        .create(&local_admin(), IngestionKeyRequest::default(), now)
        .await
        .unwrap();
    let expired = service
        .create(
            &local_admin(),
            IngestionKeyRequest {
                expires_at: Some(now + Duration::seconds(1)),
                ..IngestionKeyRequest::default()
            },
            now,
        )
        .await
        .unwrap();
    let revoked = service
        .create(&local_admin(), IngestionKeyRequest::default(), now)
        .await
        .unwrap();
    service
        .revoke(&local_admin(), &revoked.record.id)
        .await
        .unwrap();
    let restarted = Ingestion(isolated_database.restart().await);
    let records = restarted.list(&local_admin()).await.unwrap();
    assert_eq!(records.len(), 2);
    assert!(records.contains(&keep.record));
    assert!(records.contains(&expired.record));
    let snapshot = restarted
        .current_snapshot(now + Duration::seconds(1))
        .await
        .unwrap();
    assert_eq!(snapshot.keys.len(), 1);
    assert_eq!(snapshot.keys[0].tenant, keep.record.tenant);
    restarted
        .revoke(&local_admin(), &keep.record.id)
        .await
        .unwrap();
    let again = Ingestion(isolated_database.restart().await);
    assert!(
        again
            .current_snapshot(now + Duration::seconds(1))
            .await
            .unwrap()
            .keys
            .is_empty()
    );
    assert_eq!(
        again.list(&local_admin()).await.unwrap(),
        vec![expired.record]
    );
}

#[rstest]
#[tokio::test]
async fn unavailable_storage_does_not_claim_success() {
    let repository = IngestionKeys(ClickHouseState::new(
        Client::no_redirect_for_test(),
        Connection::parse("http://127.0.0.1:9").unwrap(),
    ));
    assert!(matches!(
        repository.list().await,
        Err(IngestionError::Store(StoreError::Unavailable(_)))
    ));
    assert!(matches!(
        repository.insert(&key("missing")).await,
        Err(IngestionError::Store(StoreError::Unavailable(_)))
    ));
    assert!(matches!(
        repository.revoke("missing").await,
        Err(IngestionError::Store(StoreError::Unavailable(_)))
    ));
}
