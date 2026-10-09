use super::*;
use rstest::fixture;

#[fixture]
async fn repeated_workload(#[future(awt)] database: Database) -> Database {
    let keys: Vec<_> = (0..100)
        .map(|index| format!("workload/{index:03}"))
        .collect();
    let references: Vec<_> = keys.iter().map(String::as_str).collect();
    for revision in 0..10 {
        let changes = database
            .store
            .read_many(&references)
            .await
            .unwrap()
            .into_iter()
            .map(|previous| {
                change(
                    previous,
                    json!({"revision": revision, "payload": "x".repeat(1024)}),
                )
            })
            .collect();
        database.store.commit(changes).await.unwrap();
    }
    database
}

#[rstest]
#[tokio::test]
async fn compaction_bounds_history_for_a_full_batch(#[future(awt)] repeated_workload: Database) {
    let database = repeated_workload;
    let keys = database.store.keys("workload/", "", 100).await.unwrap();
    let references: Vec<_> = keys.iter().map(String::as_str).collect();
    let before: Vec<_> = database
        .store
        .read_many(&references)
        .await
        .unwrap()
        .into_iter()
        .map(|snapshot| snapshot.value)
        .collect();
    assert_eq!(keys.len(), 100);
    assert_eq!(
        database
            .sql("SELECT count() FROM lens_state_blobs FINAL")
            .await
            .trim(),
        "1000"
    );
    database.store.compact(&references).await.unwrap();
    database.store.compact(&references).await.unwrap();
    assert_eq!(
        database
            .sql("SELECT count() FROM lens_state_blobs FINAL")
            .await
            .trim(),
        "100"
    );
    assert_eq!(
        database
            .store
            .read_many(&references)
            .await
            .unwrap()
            .into_iter()
            .map(|snapshot| snapshot.value)
            .collect::<Vec<_>>(),
        before
    );
}

#[rstest]
#[case::empty(0, false)]
#[case::over_limit(101, true)]
#[tokio::test]
async fn compaction_checks_batch_size_before_writing(
    #[future(awt)] database: Database,
    #[case] count: usize,
    #[case] invalid: bool,
) {
    let keys: Vec<_> = (0..count).map(|index| format!("key/{index}")).collect();
    let references: Vec<_> = keys.iter().map(String::as_str).collect();
    let result = database.store.compact(&references).await;
    assert_eq!(matches!(result, Err(Error::InvalidState)), invalid);
    assert_eq!(result.is_ok(), !invalid);
    assert_eq!(
        database
            .sql("SELECT count() FROM lens_state_blobs")
            .await
            .trim(),
        "0"
    );
    assert!(database.store.keys("", "", 1).await.unwrap().is_empty());
}

#[rstest]
#[tokio::test]
async fn compaction_keeps_current_values_and_fences_prepared_losers(
    #[future(awt)] database: Database,
) {
    database
        .store
        .commit(vec![initial("lens", json!({"result":"saved"}))])
        .await
        .unwrap();
    let before = database.store.read("lens").await.unwrap();
    let pending = database
        .store
        .prepare(vec![change(before.clone(), json!("loser"))])
        .await
        .unwrap();
    database.independent().compact(&["lens"]).await.unwrap();
    assert_eq!(
        database.store.read("lens").await.unwrap().value,
        before.value
    );
    assert_eq!(
        database
            .sql("SELECT count() FROM lens_state_blobs FINAL")
            .await
            .trim(),
        "1"
    );
    assert!(matches!(
        database.store.publish(&pending).await,
        Err(Error::StateConflict)
    ));
    let resolved = database.store.resolve(&[before.head]).await.unwrap();
    assert_eq!(resolved[0].value, before.value);
}

#[rstest]
#[tokio::test]
async fn abandoned_first_write_cannot_publish_after_reclamation(#[future(awt)] database: Database) {
    let pending = database
        .store
        .prepare(vec![initial("abandoned", json!("uncommitted"))])
        .await
        .unwrap();
    database
        .independent()
        .compact(&["abandoned"])
        .await
        .unwrap();
    assert!(
        database
            .store
            .read("abandoned")
            .await
            .unwrap()
            .value
            .is_null()
    );
    assert!(matches!(
        database.store.publish(&pending).await,
        Err(Error::StateConflict)
    ));
    assert_eq!(
        database
            .sql("SELECT count() FROM lens_state_blobs FINAL")
            .await
            .trim(),
        "1"
    );
}

#[rstest]
#[tokio::test]
async fn deleted_revision_cannot_resolve_to_changed_content(#[future(awt)] database: Database) {
    database
        .store
        .commit(vec![initial("lens", json!("old"))])
        .await
        .unwrap();
    let before = database.store.read("lens").await.unwrap();
    database
        .store
        .commit(vec![change(before.clone(), json!("new"))])
        .await
        .unwrap();
    database.store.compact(&["lens"]).await.unwrap();
    assert!(matches!(
        database.store.resolve(&[before.head]).await,
        Err(Error::StateUnavailable)
    ));
    assert_eq!(
        database.store.read("lens").await.unwrap().value,
        json!("new")
    );
}

#[rstest]
#[tokio::test]
async fn compaction_preserves_unselected_keys_and_new_writes(#[future(awt)] database: Database) {
    database
        .store
        .commit(vec![
            initial("selected", json!(1)),
            initial("retained", json!(2)),
        ])
        .await
        .unwrap();
    let retained = database.store.read("retained").await.unwrap();
    database.store.compact(&["selected"]).await.unwrap();
    assert_eq!(database.store.read("retained").await.unwrap(), retained);
    let current = database.store.read("selected").await.unwrap();
    database
        .store
        .commit(vec![change(current, json!(3))])
        .await
        .unwrap();
    database.store.compact(&["selected"]).await.unwrap();
    assert_eq!(
        database.store.read("selected").await.unwrap().value,
        json!(3)
    );
    assert_eq!(
        database
            .sql("SELECT count() FROM lens_state_blobs FINAL")
            .await
            .trim(),
        "2"
    );
}

#[rstest]
#[tokio::test]
async fn concurrent_compaction_preserves_atomic_counters(#[future(awt)] database: Database) {
    database
        .store
        .commit(vec![initial("counter", json!(0))])
        .await
        .unwrap();
    let (writes, compacted) = tokio::join!(
        increment_concurrently(&database, "counter", 12),
        database.store.compact(&["counter"]),
    );
    assert_eq!(writes.len(), 12);
    assert!(compacted.is_ok() || matches!(compacted, Err(Error::StateConflict)));
    assert_eq!(
        database.store.read("counter").await.unwrap().value,
        json!(12)
    );
    database.store.compact(&["counter"]).await.unwrap();
    assert_eq!(
        database
            .sql("SELECT count() FROM lens_state_blobs FINAL")
            .await
            .trim(),
        "1"
    );
}
