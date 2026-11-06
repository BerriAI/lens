#[path = "support/database.rs"]
mod storage_support;
mod support;

use lens_migrate::{Error, import};
use litellm_storage_clickhouse::state::{Change, Snapshot};
use rstest::rstest;
use serde_json::{Value, json};
use storage_support::{Database, database};
use support::{long_review_keys, many_lenses, plan, source};

impl Database {
    pub async fn seed(&self, key: &str, value: Value) {
        self.state.initialize(&self.keeper).await.unwrap();
        self.state
            .commit(vec![Change {
                previous: Snapshot::empty(key),
                value,
            }])
            .await
            .unwrap();
    }
}

#[rstest]
#[case::fresh(false)]
#[case::repeated(true)]
#[tokio::test]
async fn migrated_state_is_verified_and_retry_is_idempotent(
    #[future(awt)] database: Database,
    source: Value,
    #[case] repeat: bool,
) {
    let plan = plan(source).unwrap();
    import(&database.state, &plan, &database.keeper)
        .await
        .unwrap();
    let first = database.state.heads(&["migration/postgres"]).await.unwrap();
    if repeat {
        import(&database.state, &plan, &database.keeper)
            .await
            .unwrap();
    }
    assert_eq!(
        database.values(plan.records().keys()).await,
        *plan.records()
    );
    assert_eq!(
        database.state.heads(&["migration/postgres"]).await.unwrap(),
        first
    );
    assert_eq!(
        database
            .state
            .read("migration/postgres")
            .await
            .unwrap()
            .value["complete"],
        true
    );
}

#[rstest]
#[tokio::test]
async fn migration_and_resume_preserve_records_across_multiple_batches(
    #[future(awt)] database: Database,
    many_lenses: Value,
) {
    let plan = plan(many_lenses).unwrap();
    assert_eq!(plan.report().records, 281);
    import(&database.state, &plan, &database.keeper)
        .await
        .unwrap();
    import(&database.state, &plan, &database.keeper)
        .await
        .unwrap();
    assert_eq!(
        database.values(plan.records().keys()).await,
        *plan.records()
    );
}

#[rstest]
#[case::fresh(false)]
#[case::unpublished_batch(true)]
#[tokio::test]
async fn long_review_keys_import_and_resume_without_changing_the_snapshot(
    #[future(awt)] database: Database,
    long_review_keys: Value,
    #[case] interrupted: bool,
) {
    let plan = plan(long_review_keys).unwrap();
    if interrupted {
        database
            .seed(
                "migration/postgres",
                json!({"report":plan.report(),"complete":false}),
            )
            .await;
        database
            .state
            .prepare(
                plan.records()
                    .iter()
                    .take(128)
                    .map(|(key, value)| Change {
                        previous: Snapshot::empty(key),
                        value: value.clone(),
                    })
                    .collect(),
            )
            .await
            .unwrap();
    }
    import(&database.state, &plan, &database.keeper)
        .await
        .unwrap();
    let keys = plan
        .records()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let first = database.state.heads(&keys).await.unwrap();
    import(&database.state, &plan, &database.keeper)
        .await
        .unwrap();
    assert_eq!(database.state.heads(&keys).await.unwrap(), first);
    assert_eq!(
        database.values(plan.records().keys()).await,
        *plan.records()
    );
    let checkpoint = database.state.read("migration/postgres").await.unwrap();
    assert_eq!(checkpoint.value["complete"], true);
    assert_eq!(checkpoint.value["report"], json!(plan.report()));
}

#[rstest]
#[case::empty(false)]
#[case::partial(true)]
#[tokio::test]
async fn interrupted_import_resumes_the_same_snapshot(
    #[future(awt)] database: Database,
    source: Value,
    #[case] partial: bool,
) {
    let plan = plan(source).unwrap();
    database
        .seed(
            "migration/postgres",
            json!({"report":plan.report(),"complete":false}),
        )
        .await;
    if partial {
        let (key, value) = plan.records().first_key_value().unwrap();
        database.seed(key, value.clone()).await;
    }
    import(&database.state, &plan, &database.keeper)
        .await
        .unwrap();
    assert_eq!(
        database.values(plan.records().keys()).await,
        *plan.records()
    );
}

#[rstest]
#[tokio::test]
async fn checkpoint_prepare_without_publish_can_be_retried(
    #[future(awt)] database: Database,
    source: Value,
) {
    let plan = plan(source).unwrap();
    database.state.initialize(&database.keeper).await.unwrap();
    database
        .state
        .prepare(vec![Change {
            previous: Snapshot::empty("migration/postgres"),
            value: json!({"report":plan.report(),"complete":false}),
        }])
        .await
        .unwrap();
    import(&database.state, &plan, &database.keeper)
        .await
        .unwrap();
    assert_eq!(
        database.values(plan.records().keys()).await,
        *plan.records()
    );
}

#[rstest]
#[case::existing(false, false)]
#[case::new_key_after_checkpoint(true, false)]
#[case::changed_expected_value(true, true)]
#[tokio::test]
async fn existing_or_changed_target_data_is_never_overwritten(
    #[future(awt)] database: Database,
    source: Value,
    #[case] checkpoint: bool,
    #[case] expected_key: bool,
) {
    let plan = plan(source).unwrap();
    if checkpoint {
        database
            .seed(
                "migration/postgres",
                json!({"report":plan.report(),"complete":false}),
            )
            .await;
    }
    let key = if expected_key {
        plan.records().first_key_value().unwrap().0.as_str()
    } else {
        "session/existing"
    };
    let original = json!({"preserve":"existing state"});
    database.seed(key, original.clone()).await;
    assert!(matches!(
        import(&database.state, &plan, &database.keeper).await,
        Err(Error::TargetConflict)
    ));
    assert_eq!(database.state.read(key).await.unwrap().value, original);
}

#[rstest]
#[tokio::test]
async fn changed_source_is_rejected_on_resume(
    #[future(awt)] database: Database,
    mut source: Value,
) {
    let original = plan(source.clone()).unwrap();
    database
        .seed(
            "migration/postgres",
            json!({"report":original.report(),"complete":false}),
        )
        .await;
    source["lenses"][0]["data"]["settings"]["context"] = json!("Changed source");
    assert!(matches!(
        import(&database.state, &plan(source).unwrap(), &database.keeper).await,
        Err(Error::TargetConflict)
    ));
    assert_eq!(
        database
            .state
            .read("migration/postgres")
            .await
            .unwrap()
            .value["report"],
        serde_json::to_value(original.report()).unwrap()
    );
}
