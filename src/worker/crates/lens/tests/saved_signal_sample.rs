#[allow(
    dead_code,
    reason = "The shared fixture also supports other activity routes"
)]
mod activity {
    pub mod support;
}

use activity::support::{ADMIN, Database, database};
use lens_contract::{
    activity::ActivitySelection,
    investigations::Scope,
    signals::{SignalAttempt, SignalConfig},
    worker::{Execution, LensSettings, Sample},
};
use lens_signals::SignalRepository;
use litellm_lens::{SampleRequest, SourceReader};
use litellm_storage_clickhouse::signals::Signals;
use rstest::{fixture, rstest};
use serde_json::json;

const MATCH_TRACE: &str = "match 雪/'\"\\\n\0";

struct Classified {
    database: Database,
    config: SignalConfig,
    start: u64,
    end: u64,
    clear_run: Execution,
}

fn scope() -> Scope {
    Scope {
        all_teams: false,
        team_id: "alpha".into(),
        api_key_hash: "feedback-key".into(),
    }
}

impl Classified {
    async fn sample(
        &self,
        selection: &ActivitySelection,
        preview: bool,
        cursor: &str,
    ) -> Result<Sample, litellm_lens::Error> {
        SourceReader(self.database.state.clone())
            .sample(
                &scope(),
                SampleRequest {
                    selection,
                    start: self.start,
                    end: self.end,
                    offset: 0,
                    page_size: 1,
                    preview,
                    cursor,
                },
            )
            .await
    }

    fn selection(&self) -> ActivitySelection {
        let settings: LensSettings = serde_json::from_value(json!({
            "name": "Saved selection", "model": "evaluation",
            "signal_id": "custom_signal", "agent_name": "support",
            "filters": [{"key": "environment", "value": "test"}],
            "profile": "recorded:Support"
        }))
        .unwrap();
        ActivitySelection::from(&settings)
    }
}

async fn classify(
    repository: &Signals,
    config: &SignalConfig,
    execution: &Execution,
    score: f64,
    now: chrono::DateTime<chrono::Utc>,
) {
    let until = now + chrono::TimeDelta::minutes(5);
    assert!(
        repository
            .claim(execution, config, until, now)
            .await
            .unwrap()
    );
    if execution.trace_id == "pending" {
        return;
    }
    let attempt: SignalAttempt = serde_json::from_value(json!({
        "status": if execution.trace_id == "failed" { "failed" } else { "classified" },
        "scores": if execution.trace_id == "missing" { json!({}) } else {
            json!({"custom_signal": score})
        },
        "model": config.model,
        "error": if execution.trace_id == "error" { "failed to classify" } else { "" }
    }))
    .unwrap();
    repository
        .store(execution, config, until, now, &attempt)
        .await
        .unwrap();
}

#[fixture]
async fn classified(#[future(awt)] database: Database) -> Classified {
    let now = chrono::Utc::now();
    let at = now - chrono::TimeDelta::minutes(5);
    let config: SignalConfig = serde_json::from_value(json!({
        "model": "evaluation", "threshold": 0.8,
        "signals": [{"id": "custom_signal", "name": "Custom signal", "question": "Did the task fail?"}]
    }))
    .unwrap();
    let repository = Signals(database.sessions.clone());
    repository.save_config(&config).await.unwrap();
    let ids = [
        MATCH_TRACE,
        "second",
        "low",
        "failed",
        "error",
        "stale",
        "pending",
        "missing",
    ];
    let rows = ids.into_iter().map(|id| json!({
        "Timestamp": at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        "TraceId": id, "SpanId": "0101010101010101", "SpanName": id,
        "ServiceName": "fixture", "AgentName": "support", "TeamId": "alpha", "ApiKeyHash": "feedback-key",
        "Duration": 1_000_000_000_u64, "EngineReceivedMs": at.timestamp_millis() + 1000,
        "ResourceAttributes": {"environment": "test", "task_category": "Support"}
    }).to_string()).collect::<Vec<_>>().join("\n");
    let storage = &database.state.storage;
    litellm_storage_clickhouse::execute_statement(
        &storage.client,
        storage.config.storage().writer(),
        &format!(
            "INSERT INTO `{}`.otel_traces FORMAT JSONEachRow\n{rows}",
            storage.config.storage().database()
        ),
        std::time::Duration::from_secs(30),
    )
    .await
    .unwrap();
    database
        .trace_with_agent(
            MATCH_TRACE,
            "beta",
            at.timestamp_nanos_opt().unwrap(),
            "support",
        )
        .await;
    let start = (now - chrono::TimeDelta::hours(1)).timestamp_millis() as u64;
    let end = now.timestamp_millis() as u64;
    let traces = SourceReader(database.state.clone())
        .sample(
            &Scope {
                all_teams: true,
                ..scope()
            },
            SampleRequest {
                selection: &ActivitySelection::default(),
                start,
                end,
                offset: 0,
                page_size: 100,
                preview: true,
                cursor: "",
            },
        )
        .await
        .unwrap();
    let clear_run = traces
        .executions
        .iter()
        .find(|run| run.trace_id == "low")
        .unwrap()
        .clone();
    for execution in traces.executions {
        classify(
            &repository,
            &config,
            &execution,
            if execution.trace_id == "low" {
                0.4
            } else {
                0.8
            },
            now,
        )
        .await;
        if execution.trace_id == "stale" {
            classify(
                &repository,
                &SignalConfig {
                    model: "changed-model".into(),
                    ..config.clone()
                },
                &execution,
                0.8,
                now,
            )
            .await;
        }
    }
    Classified {
        database,
        config,
        start,
        end,
        clear_run,
    }
}

#[rstest]
#[case::all(100.0, None, false, 2)]
#[case::percentage(50.0, None, false, 1)]
#[case::cap(100.0, Some(1), false, 1)]
#[case::preview(50.0, None, true, 1)]
#[tokio::test]
async fn saved_signals_filter_before_sampling_and_pagination(
    #[future(awt)] classified: Classified,
    #[case] percent: f64,
    #[case] cap: Option<u64>,
    #[case] preview: bool,
    #[case] selected: i64,
) {
    let selection = ActivitySelection {
        sample_percent: percent,
        sample_size: cap.and_then(std::num::NonZeroU64::new),
        ..classified.selection()
    };
    let first = classified.sample(&selection, preview, "").await.unwrap();
    assert_eq!((first.eligible, first.selected), (2, selected));
    assert_eq!(first.executions.len(), 1);
    assert_eq!(first.executions[0].team_id, "alpha");
    assert!([MATCH_TRACE, "second"].contains(&first.executions[0].trace_id.as_str()));
    if preview || selected == 2 {
        let second = classified
            .sample(&selection, preview, first.next_cursor.as_deref().unwrap())
            .await
            .unwrap();
        assert_eq!(second.executions.len(), 1);
        assert_ne!(second.executions[0].id, first.executions[0].id);
        assert!(second.next_cursor.is_none());
    } else {
        assert!(first.next_cursor.is_none());
    }
}

#[rstest]
#[case::fewer_matches(0.9, false)]
#[case::more_matches(0.05, false)]
#[case::same_matches(0.7, false)]
#[case::removed_signal(0.8, true)]
#[tokio::test]
async fn changed_alert_config_invalidates_a_signal_sample_cursor(
    #[future(awt)] classified: Classified,
    #[case] threshold: f64,
    #[case] removed: bool,
) {
    let selection = classified.selection();
    let first = classified.sample(&selection, false, "").await.unwrap();
    Signals(classified.database.sessions.clone())
        .save_config(&SignalConfig {
            threshold,
            signals: if removed {
                vec![]
            } else {
                classified.config.signals.clone()
            },
            ..classified.config.clone()
        })
        .await
        .unwrap();
    assert!(
        classified
            .sample(&selection, false, first.next_cursor.as_deref().unwrap())
            .await
            .is_err()
    );
}

#[rstest]
#[tokio::test]
async fn changing_classifications_with_the_same_count_invalidates_the_cursor(
    #[future(awt)] classified: Classified,
) {
    let selection = classified.selection();
    let first = classified.sample(&selection, false, "").await.unwrap();
    let repository = Signals(classified.database.sessions.clone());
    let now = chrono::Utc::now() + chrono::TimeDelta::minutes(6);
    classify(
        &repository,
        &classified.config,
        &Execution {
            span_count: first.executions[0].span_count + 1,
            ..first.executions[0].clone()
        },
        0.4,
        now,
    )
    .await;
    classify(
        &repository,
        &classified.config,
        &Execution {
            span_count: classified.clear_run.span_count + 1,
            ..classified.clear_run.clone()
        },
        0.8,
        now,
    )
    .await;
    let fresh = classified.sample(&selection, false, "").await.unwrap();
    assert_eq!(
        (fresh.eligible, fresh.selected),
        (first.eligible, first.selected)
    );
    assert!(
        classified
            .sample(&selection, false, first.next_cursor.as_deref().unwrap())
            .await
            .is_err()
    );
}

#[rstest]
#[case::higher_threshold(0.9, "custom_signal", "evaluation", 0)]
#[case::lower_threshold(0.05, "custom_signal", "evaluation", 3)]
#[case::removed_signal(0.8, "removed_signal", "evaluation", 0)]
#[case::new_detector(0.8, "custom_signal", "new-model", 0)]
#[tokio::test]
async fn preview_uses_the_current_alert_config(
    #[future(awt)] classified: Classified,
    #[case] threshold: f64,
    #[case] signal: &str,
    #[case] model: &str,
    #[case] eligible: i64,
) {
    Signals(classified.database.sessions.clone())
        .save_config(&SignalConfig {
            threshold,
            model: model.into(),
            ..classified.config.clone()
        })
        .await
        .unwrap();
    let server = classified.database.serve(true).await;
    let selection = ActivitySelection {
        signal_id: Some(signal.try_into().unwrap()),
        ..classified.selection()
    };
    let response = server.client
        .post(server.url.join("/lens/admin/preview/sample").unwrap())
        .bearer_auth(ADMIN)
        .json(&json!({"selection": selection, "as_of": chrono::DateTime::from_timestamp_millis(classified.end as i64)}))
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    let result: Sample = response.json().await.unwrap();
    assert_eq!(result.eligible, eligible);
    assert_eq!(result.selected, eligible);
}

#[rstest]
#[tokio::test]
async fn explicit_runs_cannot_bypass_the_signal_filter(#[future(awt)] classified: Classified) {
    let selection = ActivitySelection::default();
    let unfiltered = classified.sample(&selection, true, "").await.unwrap();
    assert_eq!(unfiltered.eligible, 8);
    let selection = ActivitySelection {
        execution_ids: vec![classified.clear_run.id.clone()],
        ..classified.selection()
    };
    let filtered = classified.sample(&selection, true, "").await.unwrap();
    assert_eq!(filtered.eligible, 0);
    assert!(filtered.executions.is_empty());
}

#[rstest]
#[tokio::test]
async fn a_matching_signal_cannot_include_another_tenants_trace(
    #[future(awt)] classified: Classified,
) {
    let selection = ActivitySelection {
        signal_id: Some("custom_signal".try_into().unwrap()),
        ..Default::default()
    };
    let filtered = classified.sample(&selection, true, "").await.unwrap();
    assert_eq!(filtered.eligible, 2);
    assert_eq!(filtered.executions[0].team_id, "alpha");
}
