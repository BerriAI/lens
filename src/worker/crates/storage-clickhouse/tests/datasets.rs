#[path = "datasets/support.rs"]
mod support;

use chrono::{DateTime, Duration, Utc};
use lens_contract::datasets::Dataset;
use lens_datasets::{
    DatasetRepository, Evidence, Finding, ReadError, Scope, StoreError, StoredSummary,
};
use litellm_http::Client;
use litellm_storage_clickhouse::{
    Connection,
    datasets::{Datasets, Findings},
    state::ClickHouseState,
};
use rstest::rstest;
use serde_json::{Value, json};
use support::{
    Database, database, dataset, distinct_revisions, initial, insert_concurrently,
    isolated_database, revisions, same_revision, saved_at, seed_legacy_records, summaries,
};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

#[rstest]
#[tokio::test]
async fn immutable_revisions_keep_the_greatest_revision_as_latest(
    #[future(awt)] database: Database,
    saved_at: DateTime<Utc>,
) {
    let repository = database.repository();
    let oldest = dataset("dataset", 1, 1, "team-a");
    let newest = dataset("dataset", 3, 3, "team-a");
    let middle = dataset("dataset", 2, 2, "team-a");
    assert!(repository.insert(&oldest, saved_at).await.unwrap());
    assert!(
        repository
            .insert(&newest, saved_at + Duration::seconds(1))
            .await
            .unwrap()
    );
    assert!(
        repository
            .insert(&middle, saved_at + Duration::seconds(2))
            .await
            .unwrap()
    );
    assert_eq!(
        repository.get("dataset", Some(1)).await.unwrap(),
        Some(oldest)
    );
    assert_eq!(
        repository.get("dataset", Some(2)).await.unwrap(),
        Some(middle)
    );
    assert_eq!(
        repository.get("dataset", None).await.unwrap(),
        Some(newest.clone())
    );
    let summaries = repository.summaries().await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].team_id, newest.team_id);
    assert_eq!(summaries[0].summary.name, newest.name);
    assert_eq!(summaries[0].summary.agent_name, newest.agent_name);
    assert_eq!(summaries[0].summary.revision, newest.revision);
    assert_eq!(summaries[0].summary.case_count, newest.cases.len());
    assert_eq!(
        summaries[0].summary.updated_at,
        saved_at + Duration::seconds(1)
    );
}

#[rstest]
#[case::unknown_latest("unknown", None)]
#[case::unknown_revision("unknown", Some(1))]
#[case::missing_revision("dataset", Some(2))]
#[tokio::test]
async fn missing_datasets_and_revisions_return_none(
    #[future(awt)] database: Database,
    saved_at: DateTime<Utc>,
    #[case] id: &str,
    #[case] revision: Option<i64>,
) {
    let repository = database.repository();
    repository
        .insert(&dataset("dataset", 1, 1, "team-a"), saved_at)
        .await
        .unwrap();
    assert_eq!(repository.get(id, revision).await.unwrap(), None);
}

#[rstest]
#[tokio::test]
async fn duplicate_revision_preserves_original_document_and_timestamp(
    #[future(awt)] database: Database,
    saved_at: DateTime<Utc>,
) {
    let original = dataset("immutable", 1, 1, "team-a");
    let overwrite = dataset("immutable", 1, 4, "team-b");
    let repository = database.repository();
    assert!(repository.insert(&original, saved_at).await.unwrap());
    assert!(
        !repository
            .insert(&overwrite, saved_at + Duration::days(1))
            .await
            .unwrap()
    );
    assert_eq!(
        repository.get("immutable", Some(1)).await.unwrap(),
        Some(original)
    );
    let summaries = repository.summaries().await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].team_id, "team-a");
    assert_eq!(summaries[0].summary.case_count, 1);
    assert_eq!(summaries[0].summary.updated_at, saved_at);
}

#[rstest]
#[tokio::test]
async fn concurrent_same_revision_has_one_winner_and_matching_summary(
    #[future(awt)] database: Database,
    saved_at: DateTime<Utc>,
    same_revision: Vec<Dataset>,
) {
    let results = insert_concurrently(&database, same_revision, saved_at).await;
    let winners: Vec<_> = results
        .iter()
        .filter(|(_, result)| matches!(result, Ok(true)))
        .collect();
    assert_eq!(winners.len(), 1, "{results:?}");
    assert_eq!(
        results
            .iter()
            .filter(|(_, result)| matches!(result, Ok(false)))
            .count(),
        15
    );
    let repository = database.repository();
    let winner = winners[0].0.clone();
    assert_eq!(
        repository.get("concurrent", None).await.unwrap(),
        Some(winner.clone())
    );
    assert_eq!(
        repository.get("concurrent", Some(1)).await.unwrap(),
        Some(winner.clone())
    );
    let summaries = repository.summaries().await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].summary.case_count, winner.cases.len());
}

#[rstest]
#[tokio::test]
async fn concurrent_distinct_revisions_preserve_every_revision(
    #[future(awt)] database: Database,
    saved_at: DateTime<Utc>,
    distinct_revisions: Vec<Dataset>,
) {
    let results = insert_concurrently(&database, distinct_revisions.clone(), saved_at).await;
    assert_eq!(
        results
            .iter()
            .filter(|(_, result)| matches!(result, Ok(true)))
            .count(),
        16,
        "{results:?}"
    );
    let repository = database.repository();
    assert_eq!(
        revisions(&repository, "concurrent", 16).await,
        distinct_revisions
            .iter()
            .cloned()
            .map(Some)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        repository.get("concurrent", None).await.unwrap(),
        distinct_revisions.last().cloned()
    );
    assert_eq!(
        repository.summaries().await.unwrap()[0].summary.revision,
        16
    );
}

#[rstest]
#[tokio::test]
async fn summary_pages_include_only_published_dataset_metadata(
    #[future(awt)] database: Database,
    summaries: Vec<StoredSummary>,
) {
    let changes = summaries
        .iter()
        .map(|entry| {
            initial(
                &format!("dataset-latest/%5B%22{}%22%5D", entry.summary.id),
                serde_json::to_value(entry).unwrap(),
            )
        })
        .collect();
    database.store.commit(changes).await.unwrap();
    let _unpublished = database
        .store
        .prepare(vec![initial(
            "dataset-latest/%5B%22abandoned%22%5D",
            serde_json::to_value(&summaries[0]).unwrap(),
        )])
        .await
        .unwrap();
    database
        .seed(
            "trace-signal/%5B%22unrelated%22%5D",
            json!({"status": "pending"}),
        )
        .await;
    assert_eq!(
        database.repository().summaries().await.unwrap(),
        summaries.into_iter().rev().collect::<Vec<_>>()
    );
}

#[rstest]
#[tokio::test]
async fn summary_timestamp_ties_keep_key_order(
    #[future(awt)] database: Database,
    saved_at: DateTime<Utc>,
) {
    let repository = database.repository();
    repository
        .insert(&dataset("z", 1, 1, "team-a"), saved_at)
        .await
        .unwrap();
    repository
        .insert(&dataset("a", 1, 1, "team-a"), saved_at)
        .await
        .unwrap();
    repository
        .insert(
            &dataset("b", 1, 1, "team-a"),
            saved_at + Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(
        repository
            .summaries()
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.summary.id)
            .collect::<Vec<_>>(),
        vec!["b", "a", "z"]
    );
}

#[rstest]
#[case::unicode(
    "λ 🦀",
    0,
    "%5B%22%CE%BB%20%F0%9F%A6%80%22%5D",
    "%5B%22%CE%BB%20%F0%9F%A6%80%22%2C0%5D"
)]
#[case::unreserved("a-b_c.d~", -1, "%5B%22a-b_c.d~%22%5D", "%5B%22a-b_c.d~%22%2C-1%5D")]
#[case::escaped(
    "a\"/\\\n",
    10,
    "%5B%22a%5C%22%2F%5C%5C%5Cn%22%5D",
    "%5B%22a%5C%22%2F%5C%5C%5Cn%22%2C10%5D"
)]
#[tokio::test]
async fn writes_match_python_record_keys_and_document_shape(
    #[future(awt)] database: Database,
    saved_at: DateTime<Utc>,
    #[case] id: &str,
    #[case] revision: i64,
    #[case] latest_suffix: &str,
    #[case] revision_suffix: &str,
) {
    let expected = dataset(id, revision, 1, "team-a");
    let repository = database.repository();
    assert!(repository.insert(&expected, saved_at).await.unwrap());
    let keys = [
        format!("dataset/{revision_suffix}"),
        format!("dataset-latest/{latest_suffix}"),
    ];
    let stored = database
        .store
        .read_many(&[&keys[0], &keys[1]])
        .await
        .unwrap();
    assert_eq!(stored[0].value, serde_json::to_value(&expected).unwrap());
    assert_eq!(
        stored[1].value,
        json!({
            "team_id": "team-a", "summary": {"id": id, "name": expected.name,
            "agent_name": expected.agent_name, "revision": revision, "case_count": 1,
            "updated_at": saved_at}
        })
    );
    assert_eq!(repository.get(id, None).await.unwrap(), Some(expected));
}

#[rstest]
#[tokio::test]
async fn legacy_documents_preserve_provenance_and_normalize_timestamps(
    #[future(awt)] database: Database,
    saved_at: DateTime<Utc>,
) {
    seed_legacy_records(&database).await;
    let repository = database.repository();
    let latest = repository.get("legacy λ", None).await.unwrap().unwrap();
    assert_eq!(
        repository.get("legacy λ", Some(3)).await.unwrap(),
        Some(latest.clone())
    );
    assert_eq!(latest.name, "Existing dataset");
    assert_eq!(latest.created_at, saved_at);
    assert_eq!(latest.cases.len(), 1);
    assert_eq!(
        latest.cases[0].messages[0].content,
        "A saved conversation λ"
    );
    assert_eq!(latest.cases[0].source.lens_id, "lens-1");
    assert_eq!(latest.cases[0].source.finding_id, "finding-1");
    assert_eq!(latest.cases[0].source.trace_ref, "ref-1");
    assert_eq!(latest.cases[0].agent_version, "v1");
    assert!(!latest.cases[0].included);
    let summaries = repository.summaries().await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].summary.updated_at, saved_at);
    let next = Dataset {
        revision: latest.revision + 1,
        ..latest
    };
    assert!(
        repository
            .insert(&next, saved_at + Duration::seconds(1))
            .await
            .unwrap()
    );
    let updated = repository.summaries().await.unwrap();
    assert_eq!(updated[0].summary.case_count, 1);
    assert_eq!(updated[0].summary.revision, 4);
}

#[rstest]
#[tokio::test]
async fn equal_revision_does_not_rewrite_a_preexisting_summary(
    #[future(awt)] database: Database,
    saved_at: DateTime<Utc>,
) {
    let original = json!({"team_id": "team-a", "summary": {
        "id": "dataset", "name": "Original", "agent_name": "", "revision": 1,
        "case_count": 0, "updated_at": saved_at
    }});
    database
        .seed("dataset-latest/%5B%22dataset%22%5D", original.clone())
        .await;
    let repository = database.repository();
    let restored = dataset("dataset", 1, 1, "team-b");
    assert!(
        repository
            .insert(&restored, saved_at + Duration::days(1))
            .await
            .unwrap()
    );
    assert_eq!(
        repository.get("dataset", None).await.unwrap(),
        Some(restored)
    );
    let stored = database
        .store
        .read("dataset-latest/%5B%22dataset%22%5D")
        .await
        .unwrap();
    assert_eq!(stored.value, original);
}

#[rstest]
#[tokio::test]
async fn lens_records_without_findings_keep_the_default_empty_list(
    #[future(awt)] database: Database,
) {
    database
        .seed("lens/%5B%22lens%22%5D", json!({"lens": {"scope": {}}}))
        .await;
    assert_eq!(
        Findings(database.store.clone())
            .get("lens", &["unknown".into()], &Scope::default())
            .await
            .unwrap(),
        vec![]
    );
}

#[rstest]
#[case::latest("dataset-latest/%5B%22broken%22%5D", None)]
#[case::revision("dataset/%5B%22broken%22%2C1%5D", Some(1))]
#[tokio::test]
async fn corrupt_documents_are_unavailable_and_never_missing(
    #[future(awt)] database: Database,
    #[case] key: &str,
    #[case] revision: Option<i64>,
) {
    database.seed(key, json!({"broken": true})).await;
    assert!(matches!(
        database.repository().get("broken", revision).await,
        Err(StoreError::Unavailable(_))
    ));
}

#[rstest]
#[tokio::test]
async fn saved_revisions_survive_owned_clickhouse_and_keeper_restart(
    #[future(awt)] isolated_database: Database,
    saved_at: DateTime<Utc>,
) {
    let older = dataset("durable", 1, 1, "team-a");
    let latest = dataset("durable", 2, 2, "team-a");
    let repository = isolated_database.repository();
    repository.insert(&older, saved_at).await.unwrap();
    repository
        .insert(&latest, saved_at + Duration::seconds(1))
        .await
        .unwrap();
    let restarted = isolated_database.restart().await;
    assert_eq!(
        restarted.get("durable", Some(1)).await.unwrap(),
        Some(older)
    );
    assert_eq!(restarted.get("durable", None).await.unwrap(), Some(latest));
    assert_eq!(restarted.summaries().await.unwrap()[0].summary.revision, 2);
}

#[rstest]
#[case::storage_conflict("Code: 395. LENS_STATE_CONFLICT", true)]
#[case::existing_head("Code: 999. (Node exists)", true)]
#[case::unavailable("Code: 999. connection lost", false)]
#[tokio::test]
async fn repository_errors_keep_conflicts_distinct_from_storage_outages(
    #[case] body: &str,
    #[case] conflict: bool,
) {
    let service = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string(body))
        .mount(&service)
        .await;
    let repository = Datasets(ClickHouseState::new(
        Client::no_redirect_for_test(),
        Connection::parse(&service.uri()).unwrap(),
    ));
    assert_eq!(
        matches!(
            repository.get("unavailable", None).await.unwrap_err(),
            StoreError::Conflict
        ),
        conflict
    );
}

#[rstest]
#[tokio::test]
async fn findings_keep_stored_order_and_only_requested_evidence(#[future(awt)] database: Database) {
    database.seed("lens/%5B%22lens%22%5D", json!({
        "lens": {"scope": {"team_id": "team-a"}, "name": "Existing Lens", "findings": [
            {"id": "second", "status": "open", "evidence": [{"execution_id": "execution-2", "span_id": "span-2", "quote": "evidence"}]},
            {"id": "ignored", "evidence": []},
            {"id": "first", "evidence": [{"execution_id": "execution-1", "span_id": "span-1"}]}
        ]}, "due_at": null
    })).await;
    let findings = Findings(database.store.clone())
        .get(
            "lens",
            &["first".into(), "second".into(), "second".into()],
            &Scope {
                all_teams: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        findings,
        vec![
            Finding {
                id: "second".into(),
                evidence: vec![Evidence {
                    execution_id: "execution-2".into(),
                    span_id: "span-2".into()
                }]
            },
            Finding {
                id: "first".into(),
                evidence: vec![Evidence {
                    execution_id: "execution-1".into(),
                    span_id: "span-1".into()
                }]
            },
        ]
    );
}

#[rstest]
#[case::missing(Value::Null)]
#[case::other_team(json!({"lens": {"scope": {"team_id": "other"}, "findings": []}}))]
#[case::all_teams_target(json!({"lens": {"scope": {"team_id": "team-a", "all_teams": true}, "findings": []}}))]
#[tokio::test]
async fn missing_or_inaccessible_lenses_are_indistinguishable(
    #[future(awt)] database: Database,
    #[case] value: Value,
) {
    database.seed("lens/%5B%22lens%22%5D", value).await;
    let result = Findings(database.store.clone())
        .get(
            "lens",
            &[],
            &Scope {
                team_id: "team-a".into(),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(result, Err(ReadError::LensNotFound)));
}

#[rstest]
#[case::missing_scope(json!({"lens": {"findings": []}}))]
#[case::invalid_scope(json!({"lens": {"scope": {"all_teams": "yes"}, "findings": []}}))]
#[case::unknown_scope_field(json!({"lens": {"scope": {"all_team": true}, "findings": []}}))]
#[case::invalid_evidence(json!({"lens": {"scope": {}, "findings": [{"id": "f", "evidence": [{}]}]}}))]
#[tokio::test]
async fn malformed_lens_metadata_fails_closed(
    #[future(awt)] database: Database,
    #[case] value: Value,
) {
    database.seed("lens/%5B%22lens%22%5D", value).await;
    let result = Findings(database.store.clone())
        .get(
            "lens",
            &["f".into()],
            &Scope {
                all_teams: true,
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(ReadError::Storage(StoreError::Unavailable(_)))
    ));
}
