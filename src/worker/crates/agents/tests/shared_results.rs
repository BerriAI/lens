use lens_agents::{Error, model::*, store::Store};
use rstest::{fixture, rstest};
use serde_json::json;
use tempfile::TempDir;

#[fixture]
fn directory() -> TempDir {
    tempfile::tempdir().unwrap()
}

#[fixture]
fn actor() -> Actor {
    Actor {
        workspace: "team:one".into(),
        producer_id: "failure-bot".into(),
        execution_id: "run-one".into(),
    }
}

#[fixture]
fn definition() -> DefinitionInput {
    DefinitionInput {
        key: "task_outcome".into(),
        description: "Whether the user's requested task completed".into(),
        kind: ValueKind::String,
        allowed_values: vec![json!("succeeded"), json!("failed"), json!("unknown")],
    }
}

fn judgment(id: &str, trace_ref: &str, value: &str) -> AnnotationInput {
    AnnotationInput {
        trace: TraceIdentity {
            trace_id: "trace-one".into(),
            trace_ref: trace_ref.into(),
        },
        definition_id: id.into(),
        value: json!(value),
        evidence: "The final response says the requested task was not completed".into(),
    }
}

#[rstest]
#[tokio::test]
async fn concurrent_bots_share_one_definition(
    directory: TempDir,
    actor: Actor,
    definition: DefinitionInput,
) {
    let store = Store::open(&directory.path().join("agents.sqlite"))
        .await
        .unwrap();
    let second = Actor {
        producer_id: "other-bot".into(),
        ..actor.clone()
    };
    let (first, other) = tokio::join!(
        store.create_definition(&actor, definition.clone()),
        store.create_definition(&second, definition)
    );
    assert_eq!(first.unwrap().id, other.unwrap().id);
    assert_eq!(
        store
            .definitions(&actor.workspace, "requested task", "")
            .await
            .unwrap()
            .len(),
        1
    );
}

#[rstest]
#[tokio::test]
async fn reprocessing_is_idempotent_and_main_agent_can_filter_by_producer(
    directory: TempDir,
    actor: Actor,
    definition: DefinitionInput,
) {
    let path = directory.path().join("agents.sqlite");
    let store = Store::open(&path).await.unwrap();
    let definition = store.create_definition(&actor, definition).await.unwrap();
    let input = judgment(&definition.id, "project-one:run-one", "failed");
    store
        .annotate(&actor, std::slice::from_ref(&input))
        .await
        .unwrap();
    store
        .annotate(
            &Actor {
                execution_id: "run-two".into(),
                ..actor.clone()
            },
            &[input],
        )
        .await
        .unwrap();
    store
        .annotate(
            &Actor {
                producer_id: "other-bot".into(),
                ..actor.clone()
            },
            &[judgment(&definition.id, "project-one:run-one", "succeeded")],
        )
        .await
        .unwrap();
    drop(store);
    let store = Store::open(&path).await.unwrap();
    let result = store
        .annotations(
            &actor.workspace,
            &AnnotationFilter {
                definition_id: Some(definition.id),
                producer_id: Some(actor.producer_id),
                value: Some(json!("failed")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(result.annotations.len(), 1);
    assert_eq!(result.annotations[0].execution_id, "run-two");
    assert_eq!(
        store
            .annotations(&actor.workspace, &AnnotationFilter::default())
            .await
            .unwrap()
            .annotations
            .len(),
        2
    );
}

#[rstest]
#[tokio::test]
async fn an_invalid_batch_leaves_no_partial_classifications(
    directory: TempDir,
    actor: Actor,
    definition: DefinitionInput,
) {
    let store = Store::open(&directory.path().join("agents.sqlite"))
        .await
        .unwrap();
    let definition = store.create_definition(&actor, definition).await.unwrap();
    let outcome = store
        .annotate(
            &actor,
            &[
                judgment(&definition.id, "run-one", "failed"),
                judgment(&definition.id, "run-two", "broken"),
            ],
        )
        .await;
    assert!(matches!(outcome, Err(Error::Invalid(_))));
    assert!(
        store
            .annotations(&actor.workspace, &AnnotationFilter::default())
            .await
            .unwrap()
            .annotations
            .is_empty()
    );
}

#[rstest]
#[tokio::test]
async fn trace_references_and_workspaces_do_not_collide(
    directory: TempDir,
    actor: Actor,
    definition: DefinitionInput,
) {
    let store = Store::open(&directory.path().join("agents.sqlite"))
        .await
        .unwrap();
    let definition = store.create_definition(&actor, definition).await.unwrap();
    store
        .annotate(
            &actor,
            &[
                judgment(&definition.id, "scope-one", "failed"),
                judgment(&definition.id, "scope-two", "succeeded"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .annotations(
                &actor.workspace,
                &AnnotationFilter {
                    trace_ref: Some("scope-one".into()),
                    ..Default::default()
                }
            )
            .await
            .unwrap()
            .annotations[0]
            .annotation
            .value,
        json!("failed")
    );
    assert!(
        store
            .annotations("team:two", &AnnotationFilter::default())
            .await
            .unwrap()
            .annotations
            .is_empty()
    );
    assert!(matches!(
        store.definition("team:two", &definition.id).await,
        Err(Error::NotFound)
    ));
}

#[rstest]
#[tokio::test]
async fn definitions_cannot_silently_change_meaning(
    directory: TempDir,
    actor: Actor,
    definition: DefinitionInput,
) {
    let store = Store::open(&directory.path().join("agents.sqlite"))
        .await
        .unwrap();
    let original = store
        .create_definition(&actor, definition.clone())
        .await
        .unwrap();
    let changed = DefinitionInput {
        description: "Whether any tool call failed".into(),
        ..definition
    };
    assert!(matches!(
        store.create_definition(&actor, changed).await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(
        store
            .definition(&actor.workspace, "task_outcome")
            .await
            .unwrap()
            .definition,
        original.definition
    );
}

#[rstest]
#[tokio::test]
async fn aliases_consolidate_definitions_without_breaking_old_ids(
    directory: TempDir,
    actor: Actor,
    definition: DefinitionInput,
) {
    let store = Store::open(&directory.path().join("agents.sqlite"))
        .await
        .unwrap();
    let canonical = store
        .create_definition(&actor, definition.clone())
        .await
        .unwrap();
    let duplicate = store
        .create_definition(
            &actor,
            DefinitionInput {
                key: "run_outcome".into(),
                ..definition
            },
        )
        .await
        .unwrap();
    store
        .annotate(&actor, &[judgment(&duplicate.id, "run-one", "failed")])
        .await
        .unwrap();
    store
        .alias(&actor.workspace, "run_outcome", &canonical.id)
        .await
        .unwrap();
    let result = store
        .annotations(
            &actor.workspace,
            &AnnotationFilter {
                definition_id: Some(duplicate.id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(result.annotations[0].annotation.definition_id, canonical.id);
    assert_eq!(
        store
            .definitions(&actor.workspace, "", "")
            .await
            .unwrap()
            .len(),
        1
    );
}

#[rstest]
#[tokio::test]
async fn conflicting_aliases_preserve_both_judgments(
    directory: TempDir,
    actor: Actor,
    definition: DefinitionInput,
) {
    let store = Store::open(&directory.path().join("agents.sqlite"))
        .await
        .unwrap();
    let canonical = store
        .create_definition(&actor, definition.clone())
        .await
        .unwrap();
    let duplicate = store
        .create_definition(
            &actor,
            DefinitionInput {
                key: "run_outcome".into(),
                ..definition
            },
        )
        .await
        .unwrap();
    store
        .annotate(
            &actor,
            &[
                judgment(&canonical.id, "run-one", "failed"),
                judgment(&duplicate.id, "run-one", "succeeded"),
            ],
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .alias(&actor.workspace, "run_outcome", &canonical.id)
            .await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(
        store
            .annotations(&actor.workspace, &AnnotationFilter::default())
            .await
            .unwrap()
            .annotations
            .len(),
        2
    );
}

#[rstest]
#[tokio::test]
async fn reports_keep_provenance_and_trace_links(directory: TempDir, actor: Actor) {
    let store = Store::open(&directory.path().join("agents.sqlite"))
        .await
        .unwrap();
    let trace = TraceIdentity {
        trace_id: "trace-one".into(),
        trace_ref: "scope-one".into(),
    };
    let report = ReportInput {
        artifact_ids: vec![],
        id: "investigation-one".into(),
        title: "Unfinished export".into(),
        markdown: "The export failed after authentication expired".into(),
        traces: vec![trace.clone()],
    };
    store.save_report(&actor, &report).await.unwrap();
    let other = Actor {
        producer_id: "other-bot".into(),
        ..actor.clone()
    };
    assert!(matches!(
        store.save_report(&other, &report).await,
        Err(Error::Conflict(_))
    ));
    let results = store
        .reports(&actor.workspace, Some(&trace), None)
        .await
        .unwrap();
    assert_eq!(results[0].producer_id, actor.producer_id);
    assert_eq!(results[0].report.traces, vec![trace]);
    assert!(
        store
            .reports("team:two", None, None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[tokio::test]
async fn annotation_pages_keep_each_producers_judgment(
    directory: TempDir,
    actor: Actor,
    definition: DefinitionInput,
) {
    let store = Store::open(&directory.path().join("agents.sqlite"))
        .await
        .unwrap();
    let definition = store.create_definition(&actor, definition).await.unwrap();
    store
        .annotate(
            &actor,
            &[
                judgment(&definition.id, "first", "failed"),
                judgment(&definition.id, "second", "failed"),
            ],
        )
        .await
        .unwrap();
    let first = store
        .annotations(
            &actor.workspace,
            &AnnotationFilter {
                limit: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let second = store
        .annotations(
            &actor.workspace,
            &AnnotationFilter {
                limit: Some(1),
                after: first.next_cursor,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(first.annotations[0].annotation.trace.trace_ref, "first");
    assert_eq!(second.annotations[0].annotation.trace.trace_ref, "second");
    assert!(first.next_cursor.is_some());
    assert!(second.next_cursor.is_none());
}

#[rstest]
#[tokio::test]
async fn report_attachments_are_immutable_scoped_and_atomic(directory: TempDir, actor: Actor) {
    use lens_agents::artifacts::PublishArtifact;
    let store = Store::open(&directory.path().join("agents.sqlite"))
        .await
        .unwrap();
    let file = PublishArtifact {
        id: "chart-data".into(),
        path: "chart.csv".into(),
        filename: "chart.csv".into(),
        media_type: "text/csv".into(),
    };
    let content = b"outcome,count\nfailed,3\n";
    let first = store.save_artifact(&actor, &file, content).await.unwrap();
    assert_eq!(
        store
            .save_artifact(&actor, &file, content)
            .await
            .unwrap()
            .sha256,
        first.sha256
    );
    assert!(matches!(
        store.save_artifact(&actor, &file, b"changed").await,
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        store.artifact("other-workspace", &file.id).await,
        Err(Error::NotFound)
    ));
    let report = ReportInput {
        id: "report".into(),
        title: "Outcome counts".into(),
        markdown: "Full report".into(),
        traces: vec![],
        artifact_ids: vec![file.id.clone()],
    };
    store.save_report(&actor, &report).await.unwrap();
    let invalid = ReportInput {
        artifact_ids: vec!["missing-file".into()],
        ..report.clone()
    };
    assert!(matches!(
        store.save_report(&actor, &invalid).await,
        Err(Error::NotFound)
    ));
    assert_eq!(
        store
            .report(&actor.workspace, &report.id)
            .await
            .unwrap()
            .artifacts[0]
            .id,
        file.id
    );
}
