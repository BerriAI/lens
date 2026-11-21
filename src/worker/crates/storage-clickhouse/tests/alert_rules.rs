#[path = "persistence/mod.rs"]
#[allow(
    dead_code,
    reason = "Shared persistence fixture has helpers used by other test targets"
)]
mod persistence;
use chrono::Duration;
use lens_contract::alert_rules::{
    AlertRule, AlertRuleCondition, AlertRuleConfig, AlertRuleKind, AlertRuleMatch,
};
use litellm_storage_clickhouse::{alert_rules, signal_alerts};
use persistence::{Database, database, now};
use rstest::rstest;
use serde_json::json;

#[rstest]
#[case::unchanged(true, true, 5, false)]
#[case::threshold_changed(true, true, 10, true)]
#[case::disabled(true, false, 5, false)]
#[case::reenabled(false, true, 5, true)]
fn edits_only_restart_the_affected_rule(
    #[case] was_enabled: bool,
    #[case] enabled: bool,
    #[case] minutes: u32,
    #[case] restarted: bool,
) {
    let earlier = now();
    let later = earlier + Duration::minutes(1);
    let unchanged = AlertRule {
        id: "errors".into(),
        enabled: true,
        condition: AlertRuleCondition::ModelError,
    };
    let before = alert_rules::Settings::updated(
        AlertRuleConfig {
            revision: 0,
            rules: vec![
                AlertRule {
                    id: "duration".into(),
                    enabled: was_enabled,
                    condition: AlertRuleCondition::RunDuration { minutes: 5 },
                },
                unchanged.clone(),
            ],
        },
        &alert_rules::Settings::default(),
        earlier,
    );
    let after = alert_rules::Settings::updated(
        AlertRuleConfig {
            revision: 1,
            rules: vec![
                AlertRule {
                    id: "duration".into(),
                    enabled,
                    condition: AlertRuleCondition::RunDuration { minutes },
                },
                unchanged,
            ],
        },
        &before,
        later,
    );
    assert_eq!(after.activated_at.get("errors"), Some(&earlier));
    assert_eq!(
        after.activated_at.get("duration").copied(),
        enabled.then_some(if restarted { later } else { earlier })
    );
    let pending = AlertRuleMatch {
        rule_id: "duration".into(),
        kind: AlertRuleKind::RunDuration,
        trace_id: "run".into(),
        trace_ref: "scope".into(),
        span_id: "latest".into(),
        detected_at: earlier,
        duration_ms: 300_001,
        threshold_minutes: Some(5),
    };
    assert_eq!(after.eligible(&pending), enabled && !restarted);
    let deleted = alert_rules::Settings::updated(AlertRuleConfig::default(), &after, later);
    assert!(!deleted.eligible(&pending));
}

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
        rule_id: "errors".into(),
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
    let other_rule = AlertRuleMatch {
        rule_id: "another-error-rule".into(),
        ..matched.clone()
    };
    alert_rules::record(&database.store, &other_rule, now)
        .await
        .unwrap();
    assert_eq!(
        alert_rules::recent(&database.store, now - Duration::seconds(1), 10)
            .await
            .unwrap()
            .len(),
        2
    );
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
        3
    );
}
