#[path = "persistence/mod.rs"]
#[allow(
    dead_code,
    reason = "Shared persistence fixture has helpers used by other test targets"
)]
mod persistence;
use chrono::Duration;
use lens_contract::alert_rules::{AlertRuleKind, AlertRuleMatch};
use litellm_storage_clickhouse::{alert_rules, signal_alerts};
use persistence::{Database, database, now};
use rstest::rstest;
use serde_json::json;

#[rstest]
#[case::in_app_only(false, 0)]
#[case::slack_enabled(true, 1)]
#[tokio::test]
async fn matches_are_durable_and_deduplicated_with_optional_notifications(
    #[future(awt)] database: Database,
    #[case] notifications: bool,
    #[case] queued: usize,
) {
    let now = now();
    database
        .seed(
            signal_alerts::SETTINGS,
            json!({
                "enabled":notifications,"signal_ids":[],"webhook":null,"generation":"example",
                "activated_at":now - Duration::hours(1)
            }),
        )
        .await;
    let matched = AlertRuleMatch {
        kind: AlertRuleKind::ModelError,
        trace_id: "same-trace".into(),
        trace_ref: "team-a".into(),
        span_id: "failed-model".into(),
        detected_at: now,
        duration_ms: 100,
        threshold_minutes: None,
    };
    let independent = database.independent();
    let (first, second) = tokio::join!(
        alert_rules::record(&database.store, &matched, now),
        alert_rules::record(&independent, &matched, now),
    );
    first.unwrap();
    second.unwrap();
    alert_rules::record(&database.independent(), &matched, now)
        .await
        .unwrap();
    let recent = alert_rules::recent(&database.store, now - Duration::seconds(1), 10)
        .await
        .unwrap();
    assert_eq!(recent, vec![matched.clone()]);
    let events = database
        .store
        .keys(signal_alerts::EVENTS, "", 10)
        .await
        .unwrap();
    assert_eq!(events.len(), queued);
    if notifications {
        let event: signal_alerts::Event =
            serde_json::from_value(database.store.read(&events[0]).await.unwrap().value).unwrap();
        assert_eq!(event.rule, Some(matched.clone()));
        assert_eq!(event.status, signal_alerts::DeliveryStatus::Pending);
    }
    let other_team = AlertRuleMatch {
        trace_ref: "team-b".into(),
        ..matched
    };
    alert_rules::record(&database.store, &other_team, now)
        .await
        .unwrap();
    assert_eq!(
        alert_rules::recent(&database.store, now - Duration::seconds(1), 10)
            .await
            .unwrap()
            .len(),
        2
    );
}
