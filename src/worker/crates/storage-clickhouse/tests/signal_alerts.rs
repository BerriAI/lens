#[allow(
    dead_code,
    unused_imports,
    reason = "Shared signal persistence fixtures"
)]
#[path = "signals/support.rs"]
mod support;

use chrono::Duration;
use lens_contract::{
    signals::{SignalAttempt, SignalConfig},
    worker::Execution,
};
use lens_signals::SignalRepository;
use litellm_storage_clickhouse::signal_alerts::{self, EVENTS, Event, SETTINGS, Settings};
use rstest::rstest;
use serde_json::json;
use support::{Database, config, database, execution, identity, now};

#[rstest]
#[case::match_found(true, 0.9, 1, 1)]
#[case::below_threshold(true, 0.6, 1, 0)]
#[case::historical(true, 0.9, -1, 0)]
#[case::disabled(false, 0.9, 1, 0)]
#[tokio::test]
async fn classification_commits_only_eligible_selected_alerts(
    #[future(awt)] database: Database,
    config: SignalConfig,
    execution: Execution,
    #[case] enabled: bool,
    #[case] score: f64,
    #[case] seconds: i64,
    #[case] expected: usize,
) {
    let now = now();
    database
        .seed(
            SETTINGS,
            serde_json::to_value(Settings {
                enabled,
                signal_ids: vec!["user_frustration".into()],
                webhook: json!({"ciphertext":"fake"}),
                generation: "test".into(),
                activated_at: Some(now),
            })
            .unwrap(),
        )
        .await;
    let repository = database.repository();
    let execution = Execution {
        start_time: (now + Duration::seconds(seconds)).to_rfc3339(),
        ..execution
    };
    let result: SignalAttempt = serde_json::from_value(json!({"status":"classified","model":config.model,"scores":{"user_frustration":score,"missing_capability":0.99}})).unwrap();
    let lease = now + Duration::minutes(5);
    assert!(
        repository
            .claim(&execution, &config, lease, now)
            .await
            .unwrap()
    );
    repository
        .store(&execution, &config, lease, now, &result)
        .await
        .unwrap();
    let keys = database.store.keys(EVENTS, "", 100).await.unwrap();
    assert_eq!(keys.len(), expected);
    assert_eq!(
        repository.traces(&[identity(&execution)]).await.unwrap()[0].data["scores"]["user_frustration"],
        score
    );
    let repeated = signal_alerts::enqueue(
        &database.independent(),
        &execution,
        &config,
        &result,
        now + Duration::seconds(1),
    )
    .await
    .unwrap();
    assert!(repeated.is_empty());
    if expected > 0 {
        let event: Event =
            serde_json::from_value(database.store.read(&keys[0]).await.unwrap().value).unwrap();
        assert_eq!(event.signal_id, "user_frustration");
        assert_eq!(event.trace_ref, execution.trace_ref);
        assert_eq!(event.score, score);
    }
}
