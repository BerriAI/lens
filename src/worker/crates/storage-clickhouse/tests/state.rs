#[path = "state/support.rs"]
mod support;

use litellm_storage_clickhouse::{
    Error,
    state::{Head, Snapshot},
};
use rstest::rstest;
use serde_json::{Value, json};
use support::{
    Database, change, claim_concurrently, database, increment_concurrently, initial,
    isolated_database,
};

#[rstest]
#[tokio::test]
async fn concurrent_claims_have_one_winner(#[future(awt)] database: Database) {
    database
        .store
        .commit(vec![initial("investigation", json!({"worker": null}))])
        .await
        .unwrap();
    let queued = database.store.read("investigation").await.unwrap();
    let results = claim_concurrently(&database, queued.clone(), 16).await;
    let winners: Vec<_> = results
        .iter()
        .filter(|(_, result)| result.is_ok())
        .map(|(worker, _)| *worker)
        .collect();
    assert_eq!(winners.len(), 1, "{results:?}");
    assert_eq!(
        results
            .iter()
            .filter(|(_, result)| matches!(result, Err(Error::StateConflict)))
            .count(),
        15
    );
    let stored = database.store.read("investigation").await.unwrap();
    assert_eq!(stored.value, json!({"worker": winners[0]}));
    assert_eq!(stored.head.revision, queued.head.revision + 1);
}

#[rstest]
#[tokio::test]
async fn stale_progress_cannot_publish_its_checkpoint(#[future(awt)] database: Database) {
    database
        .store
        .commit(vec![
            initial("lens", json!("old")),
            initial("review", json!("original")),
        ])
        .await
        .unwrap();
    let before = database.store.read_many(&["lens", "review"]).await.unwrap();
    let obsolete = database
        .store
        .prepare(vec![
            change(before[0].clone(), json!("stale progress")),
            change(before[1].clone(), json!("stale checkpoint")),
        ])
        .await
        .unwrap();
    database
        .independent()
        .commit(vec![change(before[0].clone(), json!("new worker"))])
        .await
        .unwrap();
    assert!(matches!(
        database.store.publish(&obsolete).await,
        Err(Error::StateConflict)
    ));
    let after = database.store.read_many(&["lens", "review"]).await.unwrap();
    assert_eq!(after[0].value, json!("new worker"));
    assert_eq!(after[1], before[1]);
}

#[rstest]
#[tokio::test]
async fn publication_is_atomic_and_repeating_it_cannot_advance_state(
    #[future(awt)] database: Database,
) {
    let prepared = database
        .store
        .prepare(vec![
            initial("lens", json!({"completed": 1})),
            initial("review", json!({"content": "saved"})),
        ])
        .await
        .unwrap();
    assert_eq!(
        database
            .independent()
            .read_many(&["lens", "review"])
            .await
            .unwrap(),
        vec![Snapshot::empty("lens"), Snapshot::empty("review")]
    );
    database.store.publish(&prepared).await.unwrap();
    let first = database
        .independent()
        .read_many(&["lens", "review"])
        .await
        .unwrap();
    assert_eq!(first[0].value, json!({"completed": 1}));
    assert_eq!(first[1].value, json!({"content": "saved"}));
    assert!(matches!(
        database.store.publish(&prepared).await,
        Err(Error::StateConflict)
    ));
    assert_eq!(
        database.store.read_many(&["lens", "review"]).await.unwrap(),
        first
    );
}

#[rstest]
#[case::text(json!({"content": "trace evidence λ\n".repeat(150000)}))]
#[case::scalars(json!([null, false, 0, -5, 0.25, 1e-7, 1e30, "λ\n\\\"", {"a": [1, 2]}]))]
#[tokio::test]
async fn payload_roundtrip_and_historical_resolve(
    #[future(awt)] database: Database,
    #[case] value: Value,
) {
    let key = "key 'quoted' \\ λ\n";
    database
        .store
        .commit(vec![initial(key, value.clone())])
        .await
        .unwrap();
    let original = database.store.read(key).await.unwrap();
    assert_eq!(original.value, value);
    assert_eq!(original.head.revision, 1);
    database
        .store
        .commit(vec![change(original.clone(), json!({"new": true}))])
        .await
        .unwrap();
    let next = database.store.read(key).await.unwrap();
    assert_eq!(
        database
            .independent()
            .resolve(&[original.head.clone(), next.head.clone()])
            .await
            .unwrap(),
        vec![original, next]
    );
    assert_eq!(
        database
            .sql("SELECT max(length(digest)) FROM lens_state_heads")
            .await
            .trim(),
        "64"
    );
}

#[rstest]
#[tokio::test]
async fn tombstones_fence_stale_writes_and_keys_page_over_prepared_records(
    #[future(awt)] database: Database,
) {
    database
        .store
        .commit(vec![
            initial("test/a", json!(true)),
            initial("test/b", json!(true)),
            initial("other/a", json!(true)),
        ])
        .await
        .unwrap();
    let live = database.store.read("test/a").await.unwrap();
    database
        .store
        .commit(vec![change(live.clone(), Value::Null)])
        .await
        .unwrap();
    let deleted = database.store.read("test/a").await.unwrap();
    assert_eq!(deleted.value, Value::Null);
    assert_eq!(deleted.head.revision, live.head.revision + 1);
    assert!(matches!(
        database.store.commit(vec![change(live, json!(true))]).await,
        Err(Error::StateConflict)
    ));
    assert_eq!(database.store.read("test/a").await.unwrap(), deleted);
    let _prepared = database
        .store
        .prepare(vec![initial("test/c", json!("not visible"))])
        .await
        .unwrap();
    assert_eq!(
        database.store.keys("test/", "", 1).await.unwrap(),
        vec!["test/a"]
    );
    assert_eq!(
        database.store.keys("test/", "test/a", 10).await.unwrap(),
        vec!["test/b", "test/c"]
    );
    assert_eq!(
        database.store.read("test/c").await.unwrap(),
        Snapshot::empty("test/c")
    );
    assert!(
        database
            .store
            .keys("test/", "", 0)
            .await
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[tokio::test]
async fn update_retries_contention_and_noop_preserves_revision(#[future(awt)] database: Database) {
    database
        .store
        .commit(vec![initial("counter", json!(0))])
        .await
        .unwrap();
    assert_eq!(
        increment_concurrently(&database, "counter", 16).await.len(),
        16
    );
    let total = database.store.read("counter").await.unwrap();
    assert_eq!(total.value, json!(16));
    assert_eq!(total.head.revision, 17);
    assert_eq!(
        database
            .store
            .update("counter", Value::clone, 40)
            .await
            .unwrap(),
        total
    );
    assert!(matches!(
        database.store.update("counter", Value::clone, 0).await,
        Err(Error::StateConflict)
    ));
}

#[rstest]
#[tokio::test]
async fn missing_payload_is_unavailable_and_never_returns_null(#[future(awt)] database: Database) {
    database
        .sql(&format!(
            "INSERT INTO lens_state_heads VALUES ('missing', 1, '{}')",
            "a".repeat(64)
        ))
        .await;
    assert!(matches!(
        database.store.read("missing").await,
        Err(Error::StateUnavailable)
    ));
    let heads = database.store.heads(&["missing"]).await.unwrap();
    assert!(matches!(
        database.store.resolve(&heads).await,
        Err(Error::StateUnavailable)
    ));
}

#[rstest]
#[tokio::test]
async fn invalid_commits_fail_before_writing(#[future(awt)] database: Database) {
    assert!(matches!(
        database.store.commit(vec![]).await,
        Err(Error::InvalidState)
    ));
    assert!(matches!(
        database
            .store
            .commit(vec![initial("x", json!(1)), initial("x", json!(2))])
            .await,
        Err(Error::InvalidState)
    ));
    let overflow = Snapshot {
        head: Head {
            key: "x".into(),
            revision: u64::MAX,
            digest: "a".repeat(64),
        },
        value: Value::Null,
    };
    assert!(matches!(
        database
            .store
            .commit(vec![change(overflow, json!(1))])
            .await,
        Err(Error::InvalidState)
    ));
    assert!(database.store.keys("", "", 100).await.unwrap().is_empty());
    assert_eq!(
        database
            .sql("SELECT count() FROM lens_state_heads")
            .await
            .trim(),
        "0"
    );
    assert_eq!(database.store.heads(&[]).await.unwrap(), vec![]);
    assert_eq!(database.store.read_many(&[]).await.unwrap(), vec![]);
    assert_eq!(database.store.resolve(&[]).await.unwrap(), vec![]);
    database
        .store
        .initialize(&format!("/state-tests/{}", database.name))
        .await
        .unwrap();
}

#[rstest]
#[case::scalars(0)]
#[case::unicode(1)]
#[case::tombstone(2)]
#[tokio::test]
async fn reads_python_payloads_and_advances_the_existing_head(
    #[future(awt)] database: Database,
    #[case] index: usize,
) {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("state/fixtures/python_payloads.json")).unwrap();
    let row = &fixtures[index];
    let head: Head = serde_json::from_value(row.clone()).unwrap();
    let value: Value = serde_json::from_str(row["data"].as_str().unwrap()).unwrap();
    database
        .sql(&format!(
            "INSERT INTO lens_state_blobs FORMAT JSONEachRow\n{row}"
        ))
        .await;
    database
        .sql(&format!(
            "INSERT INTO lens_state_heads FORMAT JSONEachRow\n{}",
            serde_json::to_string(&head).unwrap()
        ))
        .await;
    let stored = database.store.read(&head.key).await.unwrap();
    assert_eq!(
        stored,
        Snapshot {
            head: head.clone(),
            value
        }
    );
    database
        .store
        .commit(vec![change(stored.clone(), json!({"port": "rust"}))])
        .await
        .unwrap();
    let next = database.independent().read(&head.key).await.unwrap();
    assert_eq!(next.head.revision, head.revision + 1);
    assert_eq!(next.value, json!({"port": "rust"}));
    assert_eq!(database.store.resolve(&[head]).await.unwrap(), vec![stored]);
}

#[rstest]
#[tokio::test]
async fn isolated_container_restart_preserves_commits_and_fencing(
    #[future(awt)] isolated_database: Database,
) {
    let database = isolated_database;
    database
        .store
        .commit(vec![
            initial("lens", json!("published")),
            initial("review", json!("checkpoint")),
            initial("deleted", json!("old")),
        ])
        .await
        .unwrap();
    let deleted = database.store.read("deleted").await.unwrap();
    database
        .store
        .commit(vec![change(deleted.clone(), Value::Null)])
        .await
        .unwrap();
    let prepared = database
        .store
        .prepare(vec![initial("unpublished", json!("pending"))])
        .await
        .unwrap();
    let keys = ["lens", "review", "deleted", "unpublished"];
    let before = database.store.read_many(&keys).await.unwrap();
    let restarted = database.restart().await;
    restarted
        .initialize(&format!("/state-tests/{}", database.name))
        .await
        .unwrap();
    assert_eq!(restarted.read_many(&keys).await.unwrap(), before);
    assert!(matches!(
        restarted
            .commit(vec![change(deleted, json!("stale"))])
            .await,
        Err(Error::StateConflict)
    ));
    restarted.publish(&prepared).await.unwrap();
    assert_eq!(
        restarted.read("unpublished").await.unwrap().value,
        json!("pending")
    );
}
