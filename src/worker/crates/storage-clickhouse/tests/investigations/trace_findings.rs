use lens_contract::{
    feedback::TraceIdentity,
    investigations::Lens,
    worker::{Execution, ExecutionSource, Finding, Job, JobStatus, RunAssessment, Sample},
};
use lens_investigations::{LensRepository, RepositoryError, TraceFindingsRepository};
use litellm_storage_clickhouse::state::{Change, Snapshot};
use rstest::{fixture, rstest};
use serde_json::{Value, json};

use super::support::{Database, archived, database, job, lens, record_key, stored, value};

#[fixture]
fn recorded() -> Value {
    serde_json::from_str(include_str!(
        "../../../contract/tests/fixtures/investigations_public.json"
    ))
    .unwrap()
}

#[fixture]
fn execution(recorded: Value) -> Execution {
    Execution {
        trace_id: "trace 雪/'\"".into(),
        trace_ref: "a".into(),
        ..serde_json::from_value(recorded["sample"]["executions"][0].clone()).unwrap()
    }
}

#[fixture]
fn finding(recorded: Value, execution: Execution) -> Finding {
    Finding {
        occurrences: vec![execution.id],
        ..serde_json::from_value(recorded["finding"].clone()).unwrap()
    }
}

#[fixture]
fn assessed(job: Job, execution: Execution, finding: Finding) -> Job {
    Job {
        status: JobStatus::Completed,
        sample: Some(Sample {
            executions: vec![execution.clone()],
            eligible: 1,
            selected: 1,
            next_offset: None,
            next_cursor: None,
        }),
        assessments: vec![RunAssessment {
            execution_id: execution.id,
            cannot_assess: false,
            issue_checks: vec![],
            pattern_checks: vec![],
        }],
        findings: Some(vec![finding]),
        ..job
    }
}

fn identity(execution: &Execution) -> TraceIdentity {
    TraceIdentity {
        trace_id: execution.trace_id.clone(),
        trace_ref: execution.trace_ref.clone(),
    }
}

#[rstest]
#[case::completed_assessed((JobStatus::Completed, Some(false), ExecutionSource::Traces, Some(1)))]
#[case::without_assessment((JobStatus::Completed, None, ExecutionSource::Traces, None))]
#[case::cannot_assess((JobStatus::Completed, Some(true), ExecutionSource::Traces, None))]
#[case::running((JobStatus::Running, Some(false), ExecutionSource::Traces, None))]
#[case::failed((JobStatus::Failed, Some(false), ExecutionSource::Traces, None))]
#[case::requests((JobStatus::Completed, Some(false), ExecutionSource::Requests, None))]
#[tokio::test]
async fn counts_require_completed_assessable_trace_runs(
    #[future(awt)] database: Database,
    lens: Lens,
    assessed: Job,
    execution: Execution,
    #[case] options: (JobStatus, Option<bool>, ExecutionSource, Option<u64>),
) {
    let (status, assessment, source, count) = options;
    let sample = assessed.sample.as_ref().unwrap();
    let job = Job {
        status,
        sample: Some(Sample {
            executions: vec![Execution {
                source,
                ..execution.clone()
            }],
            ..sample.clone()
        }),
        assessments: assessment
            .into_iter()
            .map(|cannot_assess| RunAssessment {
                cannot_assess,
                ..assessed.assessments[0].clone()
            })
            .collect(),
        ..assessed
    };
    database
        .repository()
        .create(&Lens {
            jobs: vec![job],
            ..lens
        })
        .await
        .unwrap();
    let rows = database
        .repository()
        .trace_findings(&[identity(&execution)])
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].trace_id, execution.trace_id);
    assert_eq!(rows[0].trace_ref, execution.trace_ref);
    assert_eq!(rows[0].finding_count, count);
}

#[rstest]
#[tokio::test]
async fn trace_refs_keep_null_and_zero_distinct_and_targets_are_deduplicated(
    #[future(awt)] database: Database,
    lens: Lens,
    assessed: Job,
    execution: Execution,
) {
    let second = Execution {
        id: "second".into(),
        trace_ref: "b".into(),
        ..execution.clone()
    };
    let sample = assessed.sample.as_ref().unwrap();
    let job = Job {
        sample: Some(Sample {
            executions: vec![execution.clone(), second.clone()],
            ..sample.clone()
        }),
        assessments: vec![
            assessed.assessments[0].clone(),
            RunAssessment {
                execution_id: second.id.clone(),
                ..assessed.assessments[0].clone()
            },
        ],
        ..assessed
    };
    database
        .repository()
        .create(&Lens {
            jobs: vec![job],
            ..lens
        })
        .await
        .unwrap();
    let rows = database
        .repository()
        .trace_findings(&[
            identity(&second),
            identity(&execution),
            TraceIdentity {
                trace_ref: "c".into(),
                ..identity(&execution)
            },
            TraceIdentity {
                trace_id: "missing".into(),
                trace_ref: String::new(),
            },
            identity(&execution),
        ])
        .await
        .unwrap();
    assert_eq!(
        value(rows),
        json!([
            {"trace_id":"missing","trace_ref":"","finding_count":null},
            {"trace_id":execution.trace_id,"trace_ref":"a","finding_count":1},
            {"trace_id":execution.trace_id,"trace_ref":"b","finding_count":0},
            {"trace_id":execution.trace_id,"trace_ref":"c","finding_count":null}
        ])
    );
}

#[rstest]
#[tokio::test]
async fn finding_counts_deduplicate_ids_across_current_and_archived_runs(
    #[future(awt)] database: Database,
    lens: Lens,
    assessed: Job,
    execution: Execution,
    finding: Finding,
) {
    let older = Job {
        id: "older".into(),
        ..assessed.clone()
    };
    let current = Job {
        id: "current".into(),
        findings: Some(vec![
            finding.clone(),
            finding.clone(),
            Finding {
                id: "second-finding".into(),
                ..finding
            },
        ]),
        ..assessed
    };
    let original = Lens {
        jobs: vec![older, current.clone()],
        ..lens
    };
    let repository = database.repository();
    repository.create(&original).await.unwrap();
    repository
        .replace(
            &original,
            &Lens {
                jobs: vec![current],
                ..original.clone()
            },
        )
        .await
        .unwrap();
    let rows = repository
        .trace_findings(&[identity(&execution)])
        .await
        .unwrap();
    assert_eq!(rows[0].finding_count, Some(2));
}

#[rstest]
#[case::version_zero(0)]
#[case::later_version(23)]
#[tokio::test]
async fn imported_archives_count_findings_at_the_original_version(
    #[future(awt)] database: Database,
    lens: Lens,
    assessed: Job,
    execution: Execution,
    #[case] version: i64,
) {
    let parent = Lens { version, ..lens };
    database
        .seed(&record_key("lens", &[&parent.id]), stored(&parent))
        .await;
    database
        .seed(
            &record_key("run", &[&parent.id, &assessed.id]),
            archived(&parent, &assessed),
        )
        .await;
    let rows = database
        .repository()
        .trace_findings(&[identity(&execution)])
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].finding_count, Some(1));
}

#[rstest]
#[case::unpublished(false)]
#[case::future(true)]
#[tokio::test]
async fn trace_counts_hide_unpublished_and_future_archives(
    #[future(awt)] database: Database,
    lens: Lens,
    assessed: Job,
    execution: Execution,
    #[case] publish: bool,
) {
    let parent = Lens { version: 1, ..lens };
    let repository = database.repository();
    repository.create(&parent).await.unwrap();
    let archived_parent = Lens {
        version: if publish { 2 } else { 1 },
        ..parent.clone()
    };
    let prepared = database
        .store
        .prepare(vec![Change {
            previous: Snapshot::empty(record_key("run", &[&parent.id, &assessed.id])),
            value: archived(&archived_parent, &assessed),
        }])
        .await
        .unwrap();
    if publish {
        database.store.publish(&prepared).await.unwrap();
    }
    assert_eq!(
        repository
            .trace_findings(&[identity(&execution)])
            .await
            .unwrap()[0]
            .finding_count,
        None
    );
}

#[rstest]
#[tokio::test]
async fn trace_counts_ignore_unpublished_and_obsolete_parent_snapshots(
    #[future(awt)] database: Database,
    lens: Lens,
    assessed: Job,
    execution: Execution,
    finding: Finding,
) {
    let repository = database.repository();
    repository.create(&lens).await.unwrap();
    let previous = database
        .store
        .read(&record_key("lens", &[&lens.id]))
        .await
        .unwrap();
    let pending = Lens {
        version: lens.version + 1,
        jobs: vec![assessed.clone()],
        ..lens.clone()
    };
    let _prepared = database
        .store
        .prepare(vec![Change {
            previous,
            value: stored(&pending),
        }])
        .await
        .unwrap();
    assert_eq!(
        repository
            .trace_findings(&[identity(&execution)])
            .await
            .unwrap()[0]
            .finding_count,
        None
    );
    let initial = repository
        .replace(
            &lens,
            &Lens {
                jobs: vec![assessed.clone()],
                ..lens.clone()
            },
        )
        .await
        .unwrap();
    let replacement = Job {
        findings: Some(vec![Finding {
            id: "new-finding".into(),
            ..finding
        }]),
        ..assessed
    };
    repository
        .replace(
            &initial,
            &Lens {
                jobs: vec![replacement],
                ..initial.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        repository
            .trace_findings(&[identity(&execution)])
            .await
            .unwrap()[0]
            .finding_count,
        Some(1)
    );
}

#[rstest]
#[tokio::test]
async fn trace_counts_report_empty_targets_and_unavailable_storage(
    #[future(awt)] database: Database,
    execution: Execution,
) {
    assert!(
        database
            .repository()
            .trace_findings(&[])
            .await
            .unwrap()
            .is_empty()
    );
    database.sql("DROP TABLE lens_state_heads SYNC").await;
    assert!(matches!(
        database
            .repository()
            .trace_findings(&[identity(&execution)])
            .await,
        Err(RepositoryError::Unavailable(_))
    ));
}

#[rstest]
#[tokio::test]
async fn request_occurrences_cannot_inflate_trace_findings_in_mixed_samples(
    #[future(awt)] database: Database,
    lens: Lens,
    assessed: Job,
    execution: Execution,
    finding: Finding,
) {
    let request = Execution {
        id: "request".into(),
        source: ExecutionSource::Requests,
        ..execution.clone()
    };
    let sample = assessed.sample.as_ref().unwrap();
    let request_finding = Finding {
        id: "request-finding".into(),
        occurrences: vec![request.id.clone()],
        ..finding
    };
    let job = Job {
        sample: Some(Sample {
            executions: vec![execution.clone(), request.clone()],
            ..sample.clone()
        }),
        assessments: vec![
            assessed.assessments[0].clone(),
            RunAssessment {
                execution_id: request.id,
                ..assessed.assessments[0].clone()
            },
        ],
        findings: Some(vec![
            assessed.findings.as_ref().unwrap()[0].clone(),
            request_finding,
        ]),
        ..assessed
    };
    database
        .repository()
        .create(&Lens {
            jobs: vec![job],
            ..lens
        })
        .await
        .unwrap();
    assert_eq!(
        database
            .repository()
            .trace_findings(&[identity(&execution)])
            .await
            .unwrap()[0]
            .finding_count,
        Some(1)
    );
}

#[rstest]
#[tokio::test]
async fn assessment_for_another_execution_does_not_mark_target_assessed(
    #[future(awt)] database: Database,
    lens: Lens,
    assessed: Job,
    execution: Execution,
) {
    let unassessed = Execution {
        id: "unassessed".into(),
        trace_ref: "different".into(),
        ..execution.clone()
    };
    let sample = assessed.sample.as_ref().unwrap();
    let job = Job {
        sample: Some(Sample {
            executions: vec![execution, unassessed.clone()],
            ..sample.clone()
        }),
        ..assessed
    };
    database
        .repository()
        .create(&Lens {
            jobs: vec![job],
            ..lens
        })
        .await
        .unwrap();
    assert_eq!(
        database
            .repository()
            .trace_findings(&[identity(&unassessed)])
            .await
            .unwrap()[0]
            .finding_count,
        None
    );
}
