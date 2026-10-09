#[path = "signals/support.rs"]
mod support;

use chrono::{DateTime, Duration, Utc};
use lens_contract::{
    signals::{SignalAttempt, SignalConfig},
    worker::Execution,
};
use lens_signals::{RepositoryError, SignalRepository, config_key};
use litellm_storage_clickhouse::state::Change;
use rstest::rstest;
use serde_json::json;

use support::{
    CONFIG, Database, concurrent_claims, config, database, execution, identity, isolated_database,
    key, now, value,
};

#[rstest]
#[tokio::test]
async fn configuration_roundtrips_and_same_configuration_preserves_head(
    #[future(awt)] database: Database,
    config: SignalConfig,
) {
    assert_eq!(
        value(database.repository().get_config().await.unwrap()),
        value(SignalConfig::default())
    );
    database.repository().save_config(&config).await.unwrap();
    assert_eq!(
        value(database.repository().get_config().await.unwrap()),
        value(&config)
    );
    let previous = database.store.read(CONFIG).await.unwrap();
    assert_eq!(previous.value, value(&config));
    database.repository().save_config(&config).await.unwrap();
    assert_eq!(database.store.read(CONFIG).await.unwrap(), previous);
}

#[rstest]
#[tokio::test]
async fn concurrent_claims_have_one_durable_owner(
    #[future(awt)] database: Database,
    config: SignalConfig,
    execution: Execution,
    now: DateTime<Utc>,
) {
    let claims = concurrent_claims(&database).await;
    assert_eq!(claims.into_iter().filter(|claimed| *claimed).count(), 1);
    let records = database
        .repository()
        .traces(&[identity(&execution)])
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    let stored = &records[0];
    assert_eq!(stored.trace_id, execution.trace_id);
    assert_eq!(stored.trace_ref, execution.trace_ref);
    assert_eq!(stored.config_key, config_key(&config));
    assert_eq!(stored.span_count, execution.span_count);
    assert_eq!(stored.claimed_until, Some(now + Duration::minutes(5)));
    assert_eq!(stored.classified_at, None);
    assert_eq!(
        stored.data,
        json!({"status":"pending","scores":{},"model":config.model,"error":""})
    );
    assert_eq!(
        database.store.read(&key(&execution)).await.unwrap().value,
        value(stored)
    );
}

#[rstest]
#[tokio::test]
async fn live_lease_blocks_config_changes_at_exact_expiry(
    #[future(awt)] database: Database,
    config: SignalConfig,
    execution: Execution,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    let lease = now + Duration::minutes(5);
    assert!(
        repository
            .claim(&execution, &config, lease, now)
            .await
            .unwrap()
    );
    let previous = database.store.read(&key(&execution)).await.unwrap();
    let config = SignalConfig {
        model: "changed".into(),
        ..config
    };
    assert!(
        !repository
            .claim(&execution, &config, lease + Duration::minutes(5), lease)
            .await
            .unwrap()
    );
    assert_eq!(
        database.store.read(&key(&execution)).await.unwrap(),
        previous
    );
    assert!(
        repository
            .claim(
                &execution,
                &config,
                lease + Duration::minutes(5),
                lease + Duration::microseconds(1)
            )
            .await
            .unwrap()
    );
    assert_eq!(
        repository.traces(&[identity(&execution)]).await.unwrap()[0].config_key,
        config_key(&config)
    );
}

#[rstest]
#[case::changed_lease(false)]
#[case::changed_config(true)]
#[tokio::test]
async fn superseded_claim_cannot_overwrite_its_successor(
    #[future(awt)] database: Database,
    config: SignalConfig,
    execution: Execution,
    now: DateTime<Utc>,
    #[case] changed_config: bool,
) {
    let repository = database.repository();
    let old_lease = now + Duration::minutes(5);
    let new_start = old_lease + Duration::microseconds(1);
    let new_lease = new_start + Duration::minutes(5);
    assert!(
        repository
            .claim(&execution, &config, old_lease, now)
            .await
            .unwrap()
    );
    let successor_config = SignalConfig {
        model: if changed_config {
            "new".into()
        } else {
            config.model.clone()
        },
        ..config.clone()
    };
    assert!(
        repository
            .claim(&execution, &successor_config, new_lease, new_start)
            .await
            .unwrap()
    );
    let previous = database.store.read(&key(&execution)).await.unwrap();
    let stale: SignalAttempt =
        serde_json::from_value(json!({"status":"failed","model":"old","error":"obsolete"}))
            .unwrap();
    repository
        .store(
            &execution,
            &config,
            if changed_config { new_lease } else { old_lease },
            new_start,
            &stale,
        )
        .await
        .unwrap();
    assert_eq!(
        database.store.read(&key(&execution)).await.unwrap(),
        previous
    );
    let accepted: SignalAttempt = serde_json::from_value(json!({"status":"classified","model":successor_config.model,"scores":{"user_frustration":0.8}})).unwrap();
    repository
        .store(
            &execution,
            &successor_config,
            new_lease,
            new_start,
            &accepted,
        )
        .await
        .unwrap();
    let completed = database.store.read(&key(&execution)).await.unwrap();
    let rows = repository.traces(&[identity(&execution)]).await.unwrap();
    assert_eq!(rows[0].claimed_until, None);
    assert_eq!(rows[0].classified_at, Some(new_start));
    assert_eq!(rows[0].data, value(&accepted));
    assert_eq!(rows[0].span_count, execution.span_count);
    repository
        .store(&execution, &successor_config, new_lease, new_start, &stale)
        .await
        .unwrap();
    assert_eq!(
        database.store.read(&key(&execution)).await.unwrap(),
        completed
    );
}

#[rstest]
#[case::grown_trace(false)]
#[case::failed_retry(true)]
#[tokio::test]
async fn reclassification_intervals_are_strict_and_reset_previous_result(
    #[future(awt)] database: Database,
    config: SignalConfig,
    execution: Execution,
    now: DateTime<Utc>,
    #[case] failed: bool,
) {
    let repository = database.repository();
    let interval = Duration::minutes(if failed { 30 } else { 5 });
    let classified_at = now - interval;
    let lease = classified_at + Duration::minutes(1);
    assert!(
        repository
            .claim(&execution, &config, lease, classified_at)
            .await
            .unwrap()
    );
    let result: SignalAttempt = serde_json::from_value(json!({"status":if failed {"failed"} else {"classified"},"model":config.model,"scores":{"user_frustration":0.8},"error":"original"})).unwrap();
    repository
        .store(&execution, &config, lease, classified_at, &result)
        .await
        .unwrap();
    let next = Execution {
        span_count: if failed { 1 } else { 3 },
        ..execution
    };
    let new_lease = now + Duration::minutes(5);
    assert!(
        !repository
            .claim(&next, &config, new_lease, now)
            .await
            .unwrap()
    );
    assert!(
        repository
            .claim(&next, &config, new_lease, now + Duration::microseconds(1))
            .await
            .unwrap()
    );
    let rows = repository.traces(&[identity(&next)]).await.unwrap();
    assert_eq!(rows[0].span_count, next.span_count);
    assert_eq!(rows[0].classified_at, None);
    assert_eq!(rows[0].claimed_until, Some(new_lease));
    assert_eq!(
        rows[0].data,
        json!({"status":"pending","scores":{},"model":config.model,"error":""})
    );
}

#[rstest]
#[case::same_config(false)]
#[case::changed_config(true)]
#[tokio::test]
async fn classified_unchanged_trace_requires_changed_configuration(
    #[future(awt)] database: Database,
    config: SignalConfig,
    execution: Execution,
    now: DateTime<Utc>,
    #[case] changed_config: bool,
) {
    let repository = database.repository();
    let lease = now + Duration::minutes(5);
    assert!(
        repository
            .claim(&execution, &config, lease, now)
            .await
            .unwrap()
    );
    let result =
        serde_json::from_value(json!({"status":"classified","model":config.model})).unwrap();
    repository
        .store(&execution, &config, lease, now, &result)
        .await
        .unwrap();
    let config = SignalConfig {
        model: if changed_config {
            "new".into()
        } else {
            config.model
        },
        ..config
    };
    assert_eq!(
        repository
            .claim(
                &execution,
                &config,
                lease + Duration::minutes(60),
                now + Duration::minutes(60)
            )
            .await
            .unwrap(),
        changed_config
    );
}

#[rstest]
#[tokio::test]
async fn reads_preserve_first_requested_order_deduplicate_and_separate_references(
    #[future(awt)] database: Database,
    config: SignalConfig,
    execution: Execution,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    let second = Execution {
        trace_ref: "a-second".into(),
        ..execution.clone()
    };
    let missing = Execution {
        trace_ref: "missing".into(),
        ..execution.clone()
    };
    let lease = now + Duration::minutes(5);
    assert!(
        repository
            .claim(&execution, &config, lease, now)
            .await
            .unwrap()
    );
    assert!(
        repository
            .claim(&second, &config, lease, now)
            .await
            .unwrap()
    );
    assert!(repository.traces(&[]).await.unwrap().is_empty());
    let rows = repository
        .traces(&[
            identity(&execution),
            identity(&second),
            identity(&missing),
            identity(&execution),
        ])
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].trace_ref, execution.trace_ref);
    assert_eq!(rows[1].trace_ref, second.trace_ref);
    assert_eq!(
        repository.traces(&[identity(&second)]).await.unwrap()[0].trace_ref,
        second.trace_ref
    );
}

#[rstest]
#[case::null(json!(null))]
#[case::array(json!(["legacy"]))]
#[case::number(json!(123))]
#[tokio::test]
async fn arbitrary_legacy_data_is_preserved_and_new_configuration_can_claim_it(
    #[future(awt)] database: Database,
    config: SignalConfig,
    execution: Execution,
    now: DateTime<Utc>,
    #[case] data: serde_json::Value,
) {
    database.seed(&key(&execution), json!({"trace_id":execution.trace_id,"trace_ref":execution.trace_ref,"config_key":"legacy","span_count":2,"data":data})).await;
    let repository = database.repository();
    let rows = repository.traces(&[identity(&execution)]).await.unwrap();
    assert_eq!(rows[0].data, data);
    assert_eq!(rows[0].classified_at, None);
    assert_eq!(rows[0].claimed_until, None);
    assert!(
        repository
            .claim(&execution, &config, now + Duration::minutes(5), now)
            .await
            .unwrap()
    );
}

#[rstest]
#[case::naive("2026-03-01T12:05:00.123456")]
#[case::offset("2026-03-01T14:05:00.123456+02:00")]
#[tokio::test]
async fn legacy_lease_formats_preserve_completion_ownership(
    #[future(awt)] database: Database,
    config: SignalConfig,
    execution: Execution,
    now: DateTime<Utc>,
    #[case] lease: &str,
) {
    database.seed(&key(&execution), json!({"trace_id":execution.trace_id,"trace_ref":execution.trace_ref,"config_key":config_key(&config),"span_count":2,"claimed_until":lease,"classified_at":"2026-03-01T12:00:00.123456","data":{"status":"pending"}})).await;
    let repository = database.repository();
    assert_eq!(
        repository.traces(&[identity(&execution)]).await.unwrap()[0].classified_at,
        Some(now)
    );
    let result =
        serde_json::from_value(json!({"status":"classified","model":config.model})).unwrap();
    repository
        .store(
            &execution,
            &config,
            now + Duration::minutes(5),
            now,
            &result,
        )
        .await
        .unwrap();
    assert_eq!(
        repository.traces(&[identity(&execution)]).await.unwrap()[0].data,
        value(result)
    );
}

#[rstest]
#[tokio::test]
async fn storing_an_unknown_trace_does_not_create_a_record(
    #[future(awt)] database: Database,
    config: SignalConfig,
    execution: Execution,
    now: DateTime<Utc>,
) {
    let previous = database.store.read(&key(&execution)).await.unwrap();
    let result =
        serde_json::from_value(json!({"status":"classified","model":config.model})).unwrap();
    database
        .repository()
        .store(&execution, &config, now, now, &result)
        .await
        .unwrap();
    assert_eq!(
        database.store.read(&key(&execution)).await.unwrap(),
        previous
    );
}

#[rstest]
#[case::configuration(0)]
#[case::claim(1)]
#[case::store(2)]
#[tokio::test]
async fn failed_writes_leave_published_state_intact(
    #[future(awt)] database: Database,
    config: SignalConfig,
    execution: Execution,
    now: DateTime<Utc>,
    #[case] operation: usize,
) {
    let repository = database.repository();
    let lease = now + Duration::minutes(5);
    if operation == 2 {
        assert!(
            repository
                .claim(&execution, &config, lease, now)
                .await
                .unwrap()
        );
    }
    let record_key = if operation == 0 {
        CONFIG.to_owned()
    } else {
        key(&execution)
    };
    let previous = database.store.read(&record_key).await.unwrap();
    database.sql("ALTER TABLE lens_state_blobs ADD CONSTRAINT reject_signal CHECK key NOT LIKE 'trace-signal/%' AND key != 'signals/config'").await;
    let result = match operation {
        0 => repository.save_config(&config).await,
        1 => repository
            .claim(&execution, &config, lease, now)
            .await
            .map(|_| ()),
        _ => {
            repository
                .store(
                    &execution,
                    &config,
                    lease,
                    now,
                    &serde_json::from_value(json!({"status":"classified","model":config.model}))
                        .unwrap(),
                )
                .await
        }
    };
    assert!(matches!(result, Err(RepositoryError::Unavailable(_))));
    assert_eq!(database.store.read(&record_key).await.unwrap(), previous);
}

#[rstest]
#[case::configuration(0)]
#[case::save_configuration(1)]
#[case::traces(2)]
#[case::claim(3)]
#[case::store(4)]
#[tokio::test]
async fn unavailable_storage_is_reported_for_every_operation(
    #[future(awt)] database: Database,
    config: SignalConfig,
    execution: Execution,
    now: DateTime<Utc>,
    #[case] operation: usize,
) {
    database.sql("DROP TABLE lens_state_heads SYNC").await;
    let repository = database.repository();
    let result = match operation {
        0 => repository.get_config().await.map(|_| ()),
        1 => repository.save_config(&config).await,
        2 => repository.traces(&[identity(&execution)]).await.map(|_| ()),
        3 => repository
            .claim(&execution, &config, now, now)
            .await
            .map(|_| ()),
        _ => {
            repository
                .store(
                    &execution,
                    &config,
                    now,
                    now,
                    &serde_json::from_value(json!({"status":"classified","model":config.model}))
                        .unwrap(),
                )
                .await
        }
    };
    assert!(matches!(result, Err(RepositoryError::Unavailable(_))));
}

#[rstest]
#[case::configuration(0, json!({"unexpected":"malformed"}))]
#[case::traces(1, json!({"unexpected":"malformed"}))]
#[case::claim(2, json!({"unexpected":"malformed"}))]
#[case::store(3, json!({"unexpected":"malformed"}))]
#[case::positional_configuration(0, json!([]))]
#[case::positional_traces(1, json!(["trace", "", "config", 2, null, null, {}]))]
#[case::positional_claim(2, json!(["trace", "", "config", 2, null, null, {}]))]
#[case::positional_store(3, json!(["trace", "", "config", 2, null, null, {}]))]
#[tokio::test]
async fn malformed_records_fail_closed(
    #[future(awt)] database: Database,
    config: SignalConfig,
    execution: Execution,
    now: DateTime<Utc>,
    #[case] operation: usize,
    #[case] malformed: serde_json::Value,
) {
    let record_key = if operation == 0 {
        CONFIG.to_owned()
    } else {
        key(&execution)
    };
    database.seed(&record_key, malformed.clone()).await;
    let repository = database.repository();
    let result = match operation {
        0 => repository.get_config().await.map(|_| ()),
        1 => repository.traces(&[identity(&execution)]).await.map(|_| ()),
        2 => repository
            .claim(&execution, &config, now, now)
            .await
            .map(|_| ()),
        _ => {
            repository
                .store(
                    &execution,
                    &config,
                    now,
                    now,
                    &serde_json::from_value(json!({"status":"classified","model":config.model}))
                        .unwrap(),
                )
                .await
        }
    };
    assert!(matches!(result, Err(RepositoryError::Unavailable(_))));
    assert_eq!(
        database.store.read(&record_key).await.unwrap().value,
        malformed
    );
}

#[rstest]
#[tokio::test]
async fn unpublished_result_cannot_replace_the_current_claim(
    #[future(awt)] database: Database,
    config: SignalConfig,
    execution: Execution,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    let lease = now + Duration::minutes(5);
    assert!(
        repository
            .claim(&execution, &config, lease, now)
            .await
            .unwrap()
    );
    let previous = database.store.read(&key(&execution)).await.unwrap();
    database.store.prepare(vec![Change { previous: previous.clone(), value:json!({"trace_id":execution.trace_id,"config_key":"future","span_count":99,"data":{"status":"classified"}}) }]).await.unwrap();
    assert_eq!(
        value(repository.traces(&[identity(&execution)]).await.unwrap()[0].clone()),
        previous.value
    );
}

#[rstest]
#[tokio::test]
async fn configuration_and_results_survive_abrupt_restart(
    #[future(awt)] isolated_database: Database,
    config: SignalConfig,
    execution: Execution,
    now: DateTime<Utc>,
) {
    let repository = isolated_database.repository();
    let lease = now + Duration::minutes(5);
    repository.save_config(&config).await.unwrap();
    assert!(
        repository
            .claim(&execution, &config, lease, now)
            .await
            .unwrap()
    );
    let result = serde_json::from_value(
        json!({"status":"classified","model":config.model,"scores":{"user_frustration":0.8}}),
    )
    .unwrap();
    repository
        .store(&execution, &config, lease, now, &result)
        .await
        .unwrap();
    let before = repository.traces(&[identity(&execution)]).await.unwrap();
    let restarted = litellm_storage_clickhouse::signals::Signals(isolated_database.restart().await);
    assert_eq!(value(restarted.get_config().await.unwrap()), value(config));
    assert_eq!(
        value(restarted.traces(&[identity(&execution)]).await.unwrap()),
        value(before)
    );
}
