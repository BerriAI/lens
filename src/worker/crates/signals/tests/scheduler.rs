mod support;
use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::signals::{SignalAttemptStatus, StoredTraceSignal};
use lens_signals::{BACKLOG_SWEEP, LIVE_SWEEP, SignalSweep, config_key, run_signal_tick};
use rstest::rstest;
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicI64, Ordering},
};
use support::*;

struct Capture(std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>);
impl litellm_tracing::Sink for Capture {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target().starts_with("lens_signals")
    }
    fn emit(&self, record: &litellm_tracing::Record) {
        self.0
            .lock()
            .unwrap()
            .push(json!({"message":record.message,"fields":record.fields}));
    }
}

#[rstest]
#[case::success(false)]
#[case::failed(true)]
#[tokio::test]
async fn classification_warning_contains_only_the_sanitized_reason(
    reader: Reader,
    repository: Repository,
    completion: Completion,
    now: DateTime<Utc>,
    #[case] failed: bool,
) {
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let logger = litellm_tracing::Logger::new(Capture(events.clone()));
    let completion = Completion {
        fail: failed,
        ..completion
    };
    logger
        .instrument(run_signal_tick(
            &reader,
            Some(&repository),
            Some(&completion),
            &|| now,
            true,
            "",
            LIVE_SWEEP,
        ))
        .await
        .unwrap();
    let captured = events.lock().unwrap();
    let expected = if failed {
        vec![
            json!({"message":"Lens signal classification failed","fields":{"reason":"Lens signal model request failed"}}),
        ]
    } else {
        vec![]
    };
    assert_eq!(*captured, expected);
}

#[rstest]
#[tokio::test]
async fn capacity_accumulates_across_pages_and_revisits_the_capped_page(
    reader: Reader,
    repository: Repository,
    completion: Completion,
    now: DateTime<Utc>,
) {
    let reader = Reader {
        samples: BTreeMap::from([
            ("".into(), sample(30, Some("second"))),
            ("second".into(), sample(25, Some("third"))),
        ]),
        ..reader
    };
    let tick = run_signal_tick(
        &reader,
        Some(&repository),
        Some(&completion),
        &|| now,
        true,
        "",
        BACKLOG_SWEEP,
    )
    .await
    .unwrap();
    assert_eq!(tick.claimed, 50);
    assert_eq!(tick.cursor, "second");
    assert_eq!(reader.scans.lock().unwrap().len(), 2);
    assert_eq!(repository.claims.lock().unwrap().len(), 50);
}

#[rstest]
#[tokio::test]
async fn provider_failure_is_stored_as_a_completed_claim(
    reader: Reader,
    repository: Repository,
    completion: Completion,
    now: DateTime<Utc>,
) {
    let completion = Completion {
        fail: true,
        ..completion
    };
    let tick = run_signal_tick(
        &reader,
        Some(&repository),
        Some(&completion),
        &|| now,
        true,
        "",
        LIVE_SWEEP,
    )
    .await
    .unwrap();
    assert_eq!(tick.claimed, 1);
    let stored = repository.stores.lock().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].attempt.status, SignalAttemptStatus::Failed);
    assert_eq!(stored[0].attempt.model, repository.config.model);
}

#[rstest]
#[case::no_repository(false, true, true)]
#[case::no_completion(true, false, true)]
#[case::not_ready(true, true, false)]
#[tokio::test]
async fn missing_dependencies_keep_cursor_without_reading(
    reader: Reader,
    repository: Repository,
    completion: Completion,
    now: DateTime<Utc>,
    #[case] storage: bool,
    #[case] model: bool,
    #[case] ready: bool,
) {
    let tick = run_signal_tick(
        &reader,
        storage.then_some(&repository),
        model.then_some(&completion),
        &|| now,
        ready,
        "saved",
        LIVE_SWEEP,
    )
    .await
    .unwrap();
    assert_eq!(tick.cursor, "saved");
    assert_eq!(tick.claimed, 0);
    assert_eq!(repository.config_reads.load(Ordering::SeqCst), 0);
    assert!(reader.scans.lock().unwrap().is_empty());
}

#[rstest]
#[tokio::test]
async fn disabled_configuration_preserves_cursor(
    reader: Reader,
    repository: Repository,
    completion: Completion,
    now: DateTime<Utc>,
) {
    let repository = Repository {
        config: Default::default(),
        ..repository
    };
    let tick = run_signal_tick(
        &reader,
        Some(&repository),
        Some(&completion),
        &|| now,
        true,
        "saved",
        LIVE_SWEEP,
    )
    .await
    .unwrap();
    assert_eq!(tick.cursor, "saved");
    assert_eq!(tick.claimed, 0);
    assert!(reader.scans.lock().unwrap().is_empty());
}

#[rstest]
#[case::live(LIVE_SWEEP, 900)]
#[case::backlog(BACKLOG_SWEEP, 86400)]
#[tokio::test]
async fn sweeps_preserve_all_team_scope_settle_window_and_pagination(
    reader: Reader,
    repository: Repository,
    completion: Completion,
    now: DateTime<Utc>,
    #[case] sweep: SignalSweep,
    #[case] lookback: i64,
) {
    let tick = run_signal_tick(
        &reader,
        Some(&repository),
        Some(&completion),
        &|| now,
        true,
        "",
        sweep,
    )
    .await
    .unwrap();
    assert_eq!(tick.cursor, "");
    assert_eq!(tick.claimed, 1);
    let scans = reader.scans.lock().unwrap();
    assert_eq!(scans.len(), 1);
    let scan = &scans[0];
    assert!(scan.scope.all_teams);
    assert!(scan.scope.team_id.is_empty());
    assert!(scan.scope.api_key_hash.is_empty());
    assert_eq!(
        scan.start,
        (now - TimeDelta::seconds(lookback)).timestamp_millis()
    );
    assert_eq!(scan.end, (now - TimeDelta::seconds(15)).timestamp_millis());
    assert_eq!(scan.limit, 100);
    assert_eq!(scan.cursor, "");
    let identities = repository.identities.lock().unwrap();
    assert_eq!(identities[0][0].trace_id, "trace-0");
    assert_eq!(identities[0][0].trace_ref, "ref");
}

#[rstest]
#[case::over_capacity(51, "")]
#[case::exact_capacity(50, "next")]
#[tokio::test]
async fn full_ticks_revisit_unprocessed_candidates_and_bound_concurrency(
    reader: Reader,
    repository: Repository,
    completion: Completion,
    now: DateTime<Utc>,
    #[case] count: usize,
    #[case] cursor: &str,
) {
    let reader = Reader {
        samples: BTreeMap::from([("".into(), sample(count, Some("next")))]),
        ..reader
    };
    let tick = run_signal_tick(
        &reader,
        Some(&repository),
        Some(&completion),
        &|| now,
        true,
        "",
        BACKLOG_SWEEP,
    )
    .await
    .unwrap();
    assert_eq!(tick.claimed, 50);
    assert_eq!(tick.cursor, cursor);
    assert_eq!(reader.scans.lock().unwrap().len(), 1);
    assert_eq!(completion.maximum.load(Ordering::SeqCst), 8);
    assert_eq!(repository.stores.lock().unwrap().len(), 50);
}

#[rstest]
#[case::live(LIVE_SWEEP, 1, "next")]
#[case::backlog(BACKLOG_SWEEP, 10, "next")]
#[tokio::test]
async fn scan_respects_each_sweeps_page_cap(
    reader: Reader,
    repository: Repository,
    completion: Completion,
    now: DateTime<Utc>,
    #[case] sweep: SignalSweep,
    #[case] pages: usize,
    #[case] cursor: &str,
) {
    let reader = Reader {
        samples: BTreeMap::from([
            ("".into(), sample(0, Some("next"))),
            ("next".into(), sample(0, Some("next"))),
        ]),
        ..reader
    };
    let tick = run_signal_tick(
        &reader,
        Some(&repository),
        Some(&completion),
        &|| now,
        true,
        "",
        sweep,
    )
    .await
    .unwrap();
    assert_eq!(tick.cursor, cursor);
    assert_eq!(tick.claimed, 0);
    assert_eq!(reader.scans.lock().unwrap().len(), pages);
}

#[rstest]
#[tokio::test]
async fn fresh_claim_clock_and_exact_lease_are_passed_to_result_storage(
    reader: Reader,
    repository: Repository,
    completion: Completion,
    now: DateTime<Utc>,
) {
    let counter = AtomicI64::new(0);
    let clock = || now + TimeDelta::seconds(counter.fetch_add(1, Ordering::SeqCst));
    run_signal_tick(
        &reader,
        Some(&repository),
        Some(&completion),
        &clock,
        true,
        "",
        LIVE_SWEEP,
    )
    .await
    .unwrap();
    let claims = repository.claims.lock().unwrap();
    let claim = &claims[0];
    assert_eq!(claim.execution.trace_id, "trace-0");
    assert_eq!(claim.config, repository.config);
    assert_eq!(claim.now, now + TimeDelta::seconds(1));
    assert_eq!(claim.lease, now + TimeDelta::seconds(301));
    let stores = repository.stores.lock().unwrap();
    let store = &stores[0];
    assert_eq!(store.execution.trace_id, claim.execution.trace_id);
    assert_eq!(store.config, claim.config);
    assert_eq!(store.lease, claim.lease);
    assert_eq!(store.at, now + TimeDelta::seconds(2));
    assert_eq!(store.attempt.status, SignalAttemptStatus::Classified);
}

#[rstest]
#[case::rejected((false, false, false, 0, 0))]
#[case::claim_error((true, true, false, 0, 0))]
#[case::store_error((true, false, true, 1, 1))]
#[tokio::test]
async fn claim_and_store_failures_do_not_overclaim(
    reader: Reader,
    repository: Repository,
    completion: Completion,
    now: DateTime<Utc>,
    #[case] outcome: (bool, bool, bool, usize, usize),
) {
    let (allow, fail_claim, fail_store, claimed, calls) = outcome;
    let repository = Repository {
        allow_claim: allow,
        fail_claim,
        fail_store,
        ..repository
    };
    let tick = run_signal_tick(
        &reader,
        Some(&repository),
        Some(&completion),
        &|| now,
        true,
        "",
        LIVE_SWEEP,
    )
    .await
    .unwrap();
    assert_eq!(tick.claimed, claimed);
    assert_eq!(completion.calls.lock().unwrap().len(), calls);
    assert_eq!(repository.stores.lock().unwrap().len(), calls);
}

#[rstest]
#[case::config(true, false, false)]
#[case::rows(false, true, false)]
#[case::sample(false, false, true)]
#[tokio::test]
async fn scan_failures_propagate_without_claiming(
    reader: Reader,
    repository: Repository,
    completion: Completion,
    now: DateTime<Utc>,
    #[case] config: bool,
    #[case] rows: bool,
    #[case] sample: bool,
) {
    let repository = Repository {
        fail_config: config,
        fail_rows: rows,
        ..repository
    };
    let reader = Reader {
        fail_sample: sample,
        ..reader
    };
    assert!(
        run_signal_tick(
            &reader,
            Some(&repository),
            Some(&completion),
            &|| now,
            true,
            "",
            LIVE_SWEEP
        )
        .await
        .is_err()
    );
    assert!(repository.claims.lock().unwrap().is_empty());
}

#[rstest]
#[case::same_reference("ref", 0)]
#[case::different_reference("other", 1)]
#[tokio::test]
async fn completed_rows_match_full_trace_identity(
    reader: Reader,
    repository: Repository,
    completion: Completion,
    now: DateTime<Utc>,
    #[case] reference: &str,
    #[case] claimed: usize,
) {
    let row = StoredTraceSignal {
        trace_id: "trace-0".into(),
        trace_ref: reference.into(),
        config_key: config_key(&repository.config),
        span_count: 2,
        claimed_until: None,
        classified_at: Some(now),
        data: json!({"status":"classified"}),
    };
    let repository = Repository {
        rows: vec![row],
        ..repository
    };
    let tick = run_signal_tick(
        &reader,
        Some(&repository),
        Some(&completion),
        &|| now,
        true,
        "",
        LIVE_SWEEP,
    )
    .await
    .unwrap();
    assert_eq!(tick.claimed, claimed);
}
