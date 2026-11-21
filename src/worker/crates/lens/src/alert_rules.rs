use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method},
    routing::get,
};
use chrono::{DateTime, TimeDelta, Utc};
use lens_auth::Authentication;
use lens_contract::{
    alert_rules::{AlertRuleConfig, AlertRuleKind, AlertRuleMatch, AlertRuleOverview},
    auth::Role,
};
use litellm_storage_clickhouse::{
    alert_rules::{self as storage, SETTINGS, Settings},
    sessions::Sessions,
    state::{Change, ClickHouseState},
};
use litellm_traces_clickhouse::query::alert_rules::{
    AlertRuleCandidates, AlertRuleParams, AlertRuleRow,
};
use serde_json::json;

use crate::error::SignalAlertError as Error;

const LEASE: &str = "alert-rules/sweep";
const PAGE_SIZE: u32 = 100;

#[derive(Clone)]
pub struct AlertRules {
    authentication: Arc<Authentication<Sessions>>,
    sources: crate::SourceReader,
    store: ClickHouseState,
}

impl AlertRules {
    pub fn new(
        authentication: Arc<Authentication<Sessions>>,
        sources: crate::SourceReader,
    ) -> Self {
        Self {
            store: authentication.sessions.0.clone(),
            authentication,
            sources,
        }
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/lens/alerts/rules", get(read).put(save))
            .route("/lens/alerts/rules/overview", get(overview))
            .layer(DefaultBodyLimit::max(4096))
            .with_state(self.clone())
    }

    async fn authorize(&self, headers: &HeaderMap, method: &Method) -> Result<(), Error> {
        let identity = lens_server::auth::identity(&self.authentication, headers, method).await?;
        if identity.user_role == Role::ProxyAdmin
            || (*method == Method::GET && identity.user_role == Role::ProxyAdminViewer)
        {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }

    async fn tick(&self) -> Result<(), crate::Error> {
        if !self.sources.0.schema_ready.load(Ordering::Acquire) {
            return Ok(());
        }
        let settings = storage::settings(&self.store).await?;
        let Some(since) = [settings.duration_since, settings.errors_since]
            .into_iter()
            .flatten()
            .min()
        else {
            return Ok(());
        };
        let now = Utc::now();
        let lease = self.store.read(LEASE).await?;
        if lease.value["until"]
            .as_i64()
            .is_some_and(|until| until > now.timestamp())
        {
            return Ok(());
        }
        let cursor = lease.value["cursor"].as_str().unwrap_or("").to_owned();
        let reservation = json!({"until":now.timestamp() + 60,"cursor":cursor});
        match self
            .store
            .commit(vec![Change {
                previous: lease,
                value: reservation.clone(),
            }])
            .await
        {
            Ok(()) => (),
            Err(litellm_storage_clickhouse::Error::StateConflict) => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        let rows = self
            .sources
            .read::<AlertRuleCandidates>(&AlertRuleParams {
                since_ms: since.timestamp_millis(),
                now_ms: now.timestamp_millis(),
                duration_ms: u64::from(settings.config.run_duration_minutes.unwrap_or(0)) * 60_000,
                model_errors: settings.config.model_errors,
                after: cursor,
                limit: PAGE_SIZE,
            })
            .await?;
        let mut processed = 0;
        for row in &rows {
            if Utc::now() >= now + TimeDelta::seconds(50) {
                break;
            }
            let current = storage::settings(&self.store).await?;
            if current.config != settings.config {
                return Ok(());
            }
            let Some(started) = DateTime::from_timestamp_millis(row.start_ms) else {
                continue;
            };
            for matched in matches(row, &settings, now) {
                storage::record(&self.store, &matched, started).await?;
            }
            processed += 1;
        }
        let lease = self.store.read(LEASE).await?;
        if lease.value == reservation {
            let more = processed < rows.len() || rows.len() == PAGE_SIZE as usize;
            let cursor = if more {
                rows.get(processed.saturating_sub(1))
                    .filter(|_| processed > 0)
                    .map(|row| row.trace_ref.as_str())
                    .unwrap_or_else(|| reservation["cursor"].as_str().unwrap_or(""))
            } else {
                ""
            };
            self.store
                .commit(vec![Change {
                    previous: lease,
                    value: json!({"until": Utc::now().timestamp() + if more { 0 } else { 30 }, "cursor": cursor}),
                }])
                .await?;
        }
        Ok(())
    }

    pub async fn serve(self) {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if self.tick().await.is_err() {
                tracing::warn!("Alert rule evaluation is unavailable; will retry");
            }
        }
    }
}

fn matches(row: &AlertRuleRow, settings: &Settings, now: DateTime<Utc>) -> Vec<AlertRuleMatch> {
    [AlertRuleKind::RunDuration, AlertRuleKind::ModelError]
        .into_iter()
        .filter_map(|kind| {
            if !settings.enabled(kind)
                || !settings
                    .since(kind)
                    .is_some_and(|since| row.start_ms >= since.timestamp_millis())
            {
                return None;
            }
            let (matched, span_id, threshold_minutes) = match kind {
                AlertRuleKind::RunDuration => (
                    row.duration_ms > u64::from(settings.config.run_duration_minutes?) * 60_000,
                    &row.duration_span_id,
                    settings.config.run_duration_minutes,
                ),
                AlertRuleKind::ModelError => {
                    (!row.error_span_id.is_empty(), &row.error_span_id, None)
                }
            };
            matched.then(|| AlertRuleMatch {
                kind,
                trace_id: row.trace_id.clone(),
                trace_ref: row.trace_ref.clone(),
                span_id: span_id.clone(),
                detected_at: now,
                duration_ms: row.duration_ms,
                threshold_minutes,
            })
        })
        .collect()
}

async fn read(
    State(app): State<AlertRules>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<AlertRuleConfig>, Error> {
    app.authorize(&headers, &method).await?;
    Ok(Json(storage::settings(&app.store).await?.config))
}

async fn save(
    State(app): State<AlertRules>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<AlertRuleConfig>, Error> {
    app.authorize(&headers, &method).await?;
    let config: AlertRuleConfig =
        serde_json::from_slice(&body).map_err(|_| Error::Invalid("Invalid alert rules"))?;
    if config
        .run_duration_minutes
        .is_some_and(|minutes| minutes == 0)
    {
        return Err(Error::Invalid("Run duration must be at least 1 minute"));
    }
    let previous = app.store.read(SETTINGS).await?;
    if previous.head.revision != config.revision {
        return Err(Error::Conflict);
    }
    let before: Settings = if previous.value.is_null() {
        Settings::default()
    } else {
        serde_json::from_value(previous.value.clone())?
    };
    let now = Utc::now();
    let settings = Settings {
        duration_since: config.run_duration_minutes.and_then(|_| {
            if config.run_duration_minutes == before.config.run_duration_minutes {
                before.duration_since.or(Some(now))
            } else {
                Some(now)
            }
        }),
        errors_since: config
            .model_errors
            .then(|| before.errors_since.unwrap_or(now)),
        config,
    };
    app.store
        .commit(vec![Change {
            previous,
            value: serde_json::to_value(settings)?,
        }])
        .await?;
    Ok(Json(storage::settings(&app.store).await?.config))
}

async fn overview(
    State(app): State<AlertRules>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<AlertRuleOverview>, Error> {
    app.authorize(&headers, &method).await?;
    let since = Utc::now() - TimeDelta::hours(24);
    let mut matches = storage::recent(&app.store, since, 501).await?;
    let truncated = matches.len() > 500;
    matches.truncate(500);
    Ok(Json(AlertRuleOverview {
        since,
        limit: 500,
        truncated,
        matches,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::below(299_999, "", true, true, 1, vec![])]
    #[case::boundary(300_000, "", true, true, 1, vec![])]
    #[case::long_run(300_001, "", true, true, 1, vec![AlertRuleKind::RunDuration])]
    #[case::model_error(100, "model-span", true, true, 1, vec![AlertRuleKind::ModelError])]
    #[case::both(300_001, "model-span", true, true, 1, vec![AlertRuleKind::RunDuration, AlertRuleKind::ModelError])]
    #[case::disabled(300_001, "model-span", false, false, 1, vec![])]
    #[case::historical(300_001, "model-span", true, true, -1, vec![])]
    fn evaluates_received_evidence_without_inference(
        #[case] duration_ms: u64,
        #[case] error_span: &str,
        #[case] duration_enabled: bool,
        #[case] model_errors: bool,
        #[case] started_after_seconds: i64,
        #[case] expected: Vec<AlertRuleKind>,
    ) {
        let now = Utc::now();
        let row = AlertRuleRow {
            trace_id: "example-trace".into(),
            trace_ref: "scoped-example-trace".into(),
            start_ms: (now + TimeDelta::seconds(started_after_seconds)).timestamp_millis(),
            duration_ms,
            duration_span_id: "latest-span".into(),
            error_span_id: error_span.into(),
        };
        let settings = Settings {
            config: AlertRuleConfig {
                revision: 1,
                run_duration_minutes: duration_enabled.then_some(5),
                model_errors,
            },
            duration_since: Some(now),
            errors_since: Some(now),
        };
        let found = matches(&row, &settings, now);
        assert_eq!(
            found.iter().map(|item| item.kind).collect::<Vec<_>>(),
            expected
        );
        if let Some(first) = found.first() {
            assert_eq!(first.trace_ref, row.trace_ref);
            assert_eq!(
                first.span_id,
                if first.kind == AlertRuleKind::ModelError {
                    error_span
                } else {
                    "latest-span"
                }
            );
        }
    }
}
