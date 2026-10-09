use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    feedback::TraceIdentity,
    signals::{SignalConfig, StoredTraceSignal, TraceSignalStatus},
    worker::Execution,
};
use lens_signals::{candidate, claimable, config_key, enabled, trace_signals};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[fixture]
fn now() -> DateTime<Utc> {
    "2026-01-01T12:00:00Z".parse().unwrap()
}
#[fixture]
fn config() -> SignalConfig {
    serde_json::from_value(json!({"model":"decision","signals":[{"id":"a","name":"First","question":"First question?"},{"id":"b","name":"Second","question":"Second question?"},{"id":"c","name":"Third","question":"Third question?"}]})).unwrap()
}
#[fixture]
fn execution() -> Execution {
    serde_json::from_value(json!({"id":"id","source":"traces","trace_id":"trace","trace_ref":"ref","team_id":"team","name":"agent","start_time":"","span_count":2})).unwrap()
}

#[rstest]
fn key_uses_canonical_unicode_json_without_names_or_threshold() {
    let config:SignalConfig=serde_json::from_value(json!({"model":"雪","threshold":0.7,"signals":[{"id":"a","name":"ignored","question":"Why 雪?"}]})).unwrap();
    let expected = format!(
        "{:x}",
        Sha256::digest(r#"{"model":"雪","signals":[{"id":"a","question":"Why 雪?"}]}"#)
    );
    assert_eq!(config_key(&config), expected);
}

#[rstest]
#[case::name("name",json!("Renamed"),true)]
#[case::question("question",json!("Changed question?"),false)]
#[case::id("id",json!("changed"),false)]
fn key_tracks_definition_but_not_presentation(
    config: SignalConfig,
    #[case] field: &str,
    #[case] value: Value,
    #[case] equal: bool,
) {
    let mut changed = serde_json::to_value(&config).unwrap();
    changed["signals"][0][field] = value;
    assert_eq!(
        config_key(&config) == config_key(&serde_json::from_value(changed).unwrap()),
        equal
    );
}

#[rstest]
fn key_tracks_model_and_signal_order(config: SignalConfig) {
    assert_ne!(
        config_key(&config),
        config_key(&SignalConfig {
            model: "another".into(),
            ..config.clone()
        })
    );
    assert_ne!(
        config_key(&config),
        config_key(&SignalConfig {
            signals: config.signals.iter().cloned().rev().collect(),
            ..config.clone()
        })
    );
    assert_eq!(
        config_key(&config),
        config_key(&SignalConfig {
            threshold: 0.9,
            ..config
        })
    );
}

#[rstest]
#[case::enabled("model", true, true)]
#[case::no_model("", true, false)]
#[case::no_signals("model", false, false)]
#[case::whitespace(" ", true, true)]
fn enabled_requires_model_and_signals(
    config: SignalConfig,
    #[case] model: &str,
    #[case] signals: bool,
    #[case] expected: bool,
) {
    assert_eq!(
        enabled(&SignalConfig {
            model: model.into(),
            signals: if signals { config.signals } else { vec![] },
            threshold: 0.5
        }),
        expected
    );
}

#[rstest]
#[case::active_changed(("pending", Some(1), None, 2, true, false, false))]
#[case::equal_lease(("pending", Some(0), None, 2, false, true, false))]
#[case::expired_pending(("pending",Some(-1),None,2,false,true,true))]
#[case::missing_lease(("pending", None, None, 2, false, false, false))]
#[case::changed(("classified", None, None, 2, true, true, true))]
#[case::no_growth(("classified",None,Some(-301),2,false,false,false))]
#[case::growth_before(("classified",None,Some(-301),1,false,true,true))]
#[case::growth_boundary(("classified",None,Some(-300),1,false,false,false))]
#[case::growth_after(("classified",None,Some(-299),1,false,false,false))]
#[case::growth_no_date(("classified", None, None, 1, false, false, false))]
#[case::failed_before(("failed",None,Some(-1801),2,false,true,true))]
#[case::failed_boundary(("failed",None,Some(-1800),2,false,false,false))]
#[case::failed_after(("failed",None,Some(-1799),2,false,false,false))]
#[case::failed_no_date(("failed", None, None, 2, false, false, false))]
#[case::fewer_spans_failed(("failed",None,Some(-1801),3,false,false,true))]
#[case::fewer_spans_classified(("classified",None,Some(-1801),3,false,false,false))]
#[case::pending_growth_without_lease(("pending",None,Some(-301),1,false,false,true))]
fn eligibility_keeps_scan_and_claim_boundary_differences(
    now: DateTime<Utc>,
    config: SignalConfig,
    execution: Execution,
    #[case] row: (&str, Option<i64>, Option<i64>, i64, bool, bool, bool),
) {
    let (status, lease, date, count, changed, scan, claim) = row;
    let key = config_key(&config);
    let row = StoredTraceSignal {
        trace_id: execution.trace_id.clone(),
        trace_ref: execution.trace_ref.clone(),
        config_key: if changed { "other".into() } else { key.clone() },
        span_count: count,
        claimed_until: lease.map(|offset| now + TimeDelta::seconds(offset)),
        classified_at: date.map(|offset| now + TimeDelta::seconds(offset)),
        data: json!({"status":status}),
    };
    assert_eq!(candidate(&execution, Some(&row), &key, now), scan);
    assert_eq!(claimable(&execution, Some(&row), &key, now), claim);
}

#[rstest]
fn unseen_execution_is_eligible(execution: Execution, now: DateTime<Utc>) {
    assert!(candidate(&execution, None, "key", now));
    assert!(claimable(&execution, None, "key", now));
}

#[fixture]
fn identity() -> TraceIdentity {
    TraceIdentity {
        trace_id: "requested".into(),
        trace_ref: "requested-ref".into(),
    }
}

#[rstest]
#[case::pending(json!({"status":"pending","model":"m","error":"pending wins"}),TraceSignalStatus::Pending,false)]
#[case::failed(json!({"status":"failed","model":"m"}),TraceSignalStatus::Failed,true)]
#[case::error(json!({"status":"classified","model":"m","error":"failure"}),TraceSignalStatus::Failed,true)]
#[case::classified(json!({"status":"classified","model":"m"}),TraceSignalStatus::Classified,true)]
fn projection_keeps_status_model_and_time(
    config: SignalConfig,
    now: DateTime<Utc>,
    identity: TraceIdentity,
    #[case] data: Value,
    #[case] status: TraceSignalStatus,
    #[case] date: bool,
) {
    let row = StoredTraceSignal {
        trace_id: "stored".into(),
        trace_ref: "stored-ref".into(),
        config_key: config_key(&config),
        span_count: 2,
        claimed_until: None,
        classified_at: Some(now),
        data,
    };
    let result = trace_signals(&identity, Some(&row), &config).unwrap();
    assert_eq!(result.status, status);
    assert_eq!(result.model, "m");
    assert_eq!(result.classified_at, date.then_some(now));
    assert!(result.flags.is_empty());
    assert_eq!(result.trace_id, identity.trace_id);
    assert_eq!(result.trace_ref, identity.trace_ref);
}

#[rstest]
fn projection_sorts_scores_stably_and_uses_current_names(
    config: SignalConfig,
    identity: TraceIdentity,
) {
    let row = StoredTraceSignal {
        trace_id: "t".into(),
        trace_ref: "r".into(),
        config_key: config_key(&config),
        span_count: 2,
        claimed_until: None,
        classified_at: None,
        data: json!({"status":"classified","scores":{"a":0.5,"b":0.9,"c":0.9,"removed":1.0},
            "evidence":{"b":{"span_id":"problem-step","quote":"This still does not work"}}}),
    };
    let result = trace_signals(&identity, Some(&row), &config).unwrap();
    assert_eq!(
        result
            .flags
            .iter()
            .map(|flag| (&*flag.signal_id, &*flag.name, flag.score))
            .collect::<Vec<_>>(),
        vec![
            ("b", "Second", 0.9),
            ("c", "Third", 0.9),
            ("a", "First", 0.5)
        ]
    );
    assert_eq!(
        result.flags[0].evidence.as_ref().unwrap().span_id,
        "problem-step"
    );
    assert_eq!(
        result.flags[0].evidence.as_ref().unwrap().quote,
        "This still does not work"
    );
    assert!(result.flags[1].evidence.is_none());
    let raised = trace_signals(
        &identity,
        Some(&row),
        &SignalConfig {
            threshold: 0.6,
            ..config
        },
    )
    .unwrap();
    assert_eq!(raised.flags.len(), 2);
}

#[rstest]
#[case::absent(false)]
#[case::stale(true)]
fn stale_or_missing_rows_are_unclassified(
    config: SignalConfig,
    identity: TraceIdentity,
    now: DateTime<Utc>,
    #[case] exists: bool,
) {
    let row = StoredTraceSignal {
        trace_id: "t".into(),
        trace_ref: "r".into(),
        config_key: "stale".into(),
        span_count: 2,
        claimed_until: None,
        classified_at: Some(now),
        data: json!({"model":"old","status":"failed"}),
    };
    let result = trace_signals(&identity, exists.then_some(&row), &config).unwrap();
    assert_eq!(result.status, TraceSignalStatus::Unclassified);
    assert_eq!(result.model, "");
    assert_eq!(result.classified_at, None);
    assert!(result.flags.is_empty());
}

#[rstest]
#[case::bad_status(json!({"status":"unclassified"}))]
#[case::bad_score(json!({"status":"classified","scores":{"a":2}}))]
#[case::not_object(json!([]))]
fn malformed_stored_data_fails_projection(
    config: SignalConfig,
    identity: TraceIdentity,
    #[case] data: Value,
) {
    let row = StoredTraceSignal {
        trace_id: "t".into(),
        trace_ref: "r".into(),
        config_key: config_key(&config),
        span_count: 1,
        claimed_until: None,
        classified_at: None,
        data,
    };
    assert!(trace_signals(&identity, Some(&row), &config).is_err());
}
