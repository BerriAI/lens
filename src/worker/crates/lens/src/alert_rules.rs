use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex, atomic::Ordering},
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
    alert_rules::{
        AlertRuleCondition, AlertRuleConfig, AlertRuleKind, AlertRuleMatch, AlertRuleOverview,
    },
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
use tokio::time::Instant;

use crate::error::SignalAlertError as Error;

const LEASE: &str = "alert-rules/sweep";
const PAGE_SIZE: u32 = 100;
const SETTINGS_RELOAD: Duration = Duration::from_secs(60);
const SWEEP: Duration = Duration::from_secs(60);
const SETTLE: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct AlertRules {
    authentication: Arc<Authentication<Sessions>>,
    sources: crate::SourceReader,
    store: ClickHouseState,
    settings: Arc<Mutex<Option<(Instant, Settings)>>>,
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
            settings: Arc::default(),
        }
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/lens/admin/alerts/rules", get(read).put(save))
            .route("/lens/admin/alerts/rules/overview", get(overview))
            .layer(DefaultBodyLimit::max(32_768))
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

    fn remember(&self, settings: &Settings) {
        if let Ok(mut cache) = self.settings.lock() {
            *cache = Some((Instant::now(), settings.clone()));
        }
    }

    async fn settings(&self) -> Result<Settings, litellm_storage_clickhouse::Error> {
        let remembered = self.settings.lock().ok().and_then(|cache| {
            cache
                .as_ref()
                .filter(|(loaded, _)| loaded.elapsed() < SETTINGS_RELOAD)
                .map(|(_, settings)| settings.clone())
        });
        if let Some(settings) = remembered {
            return Ok(settings);
        }
        let settings = storage::settings(&self.store).await?;
        self.remember(&settings);
        Ok(settings)
    }

    async fn tick(&self) -> Result<bool, crate::Error> {
        if !self.sources.0.schema_ready.load(Ordering::Acquire) {
            return Ok(false);
        }
        let settings = self.settings().await?;
        let Some(since) = settings.active().map(|(_, since)| since).min() else {
            return Ok(false);
        };
        let now = Utc::now();
        let lease = self.store.read(LEASE).await?;
        if lease.value["until"]
            .as_i64()
            .is_some_and(|until| until > now.timestamp())
        {
            return Ok(false);
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
            Err(litellm_storage_clickhouse::Error::StateConflict) => return Ok(false),
            Err(error) => return Err(error.into()),
        }
        let rows = self
            .sources
            .read::<AlertRuleCandidates>(&AlertRuleParams {
                since_ms: since.timestamp_millis(),
                now_ms: now.timestamp_millis(),
                duration_ms: u64::from(
                    settings
                        .active()
                        .filter_map(|(rule, _)| rule.condition.duration_minutes())
                        .min()
                        .unwrap_or(0),
                ) * 60_000,
                model_errors: settings
                    .active()
                    .any(|(rule, _)| rule.condition.kind() == AlertRuleKind::ModelError),
                after: cursor,
                limit: PAGE_SIZE,
            })
            .await?;
        let mut processed = 0;
        for row in &rows {
            if Utc::now() >= now + TimeDelta::seconds(50) {
                break;
            }
            if self.settings().await?.config != settings.config {
                break;
            }
            let Some(started) = DateTime::from_timestamp_millis(row.start_ms) else {
                continue;
            };
            for matched in matches(row, &settings, now) {
                storage::record(&self.store, &matched, started).await?;
            }
            processed += 1;
        }
        let more = processed < rows.len() || rows.len() == PAGE_SIZE as usize;
        let lease = self.store.read(LEASE).await?;
        if lease.value == reservation {
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
                    value: json!({"until": 0, "cursor": cursor}),
                }])
                .await?;
        }
        Ok(more)
    }

    pub async fn serve(self) {
        let mut wake = self.sources.0.wake.listen();
        wake.mark_changed();
        loop {
            crate::wake::arrival(&mut wake, SETTLE, SWEEP).await;
            loop {
                match self.tick().await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(_) => {
                        tracing::warn!("Alert rule evaluation is unavailable; will retry");
                        break;
                    }
                }
            }
        }
    }
}

fn matches(row: &AlertRuleRow, settings: &Settings, now: DateTime<Utc>) -> Vec<AlertRuleMatch> {
    settings
        .active()
        .filter_map(|(rule, since)| {
            if row.start_ms < since.timestamp_millis() {
                return None;
            }
            let (matched, span_id, threshold_minutes) = match rule.condition {
                AlertRuleCondition::RunDuration { minutes } => (
                    row.duration_ms > u64::from(minutes) * 60_000,
                    &row.duration_span_id,
                    Some(minutes),
                ),
                AlertRuleCondition::ModelError => {
                    (!row.error_span_id.is_empty(), &row.error_span_id, None)
                }
            };
            matched.then(|| AlertRuleMatch {
                rule_id: rule.id.clone(),
                kind: rule.condition.kind(),
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
    if config.rules.len() > 100 {
        return Err(Error::Invalid("Save up to 100 alert rules"));
    }
    let ids: BTreeSet<_> = config.rules.iter().map(|rule| &rule.id).collect();
    if ids.len() != config.rules.len()
        || config.rules.iter().any(|rule| {
            rule.id.is_empty()
                || rule.id.len() > 64
                || !rule
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        })
    {
        return Err(Error::Invalid(
            "Each rule needs a unique ID with up to 64 letters, numbers, hyphens or underscores",
        ));
    }
    if config
        .rules
        .iter()
        .any(|rule| rule.condition.duration_minutes() == Some(0))
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
    let settings = Settings::updated(config, &before, now);
    app.store
        .commit(vec![Change {
            previous,
            value: serde_json::to_value(settings)?,
        }])
        .await?;
    let settings = storage::settings(&app.store).await?;
    app.remember(&settings);
    app.sources.0.wake.notify();
    Ok(Json(settings.config))
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
    use lens_contract::alert_rules::AlertRule;
    use rstest::rstest;

    #[rstest]
    #[case::first_boundary(300_000, vec![])]
    #[case::first_match(300_001, vec!["short"])]
    #[case::second_boundary(1_200_000, vec!["short"])]
    #[case::both(1_200_001, vec!["short", "long"])]
    fn evaluates_each_duration_rule_independently(
        #[case] duration_ms: u64,
        #[case] expected: Vec<&str>,
    ) {
        let now = Utc::now();
        let settings = Settings::updated(
            AlertRuleConfig {
                revision: 0,
                rules: vec![
                    AlertRule {
                        id: "short".into(),
                        enabled: true,
                        condition: AlertRuleCondition::RunDuration { minutes: 5 },
                    },
                    AlertRule {
                        id: "long".into(),
                        enabled: true,
                        condition: AlertRuleCondition::RunDuration { minutes: 20 },
                    },
                ],
            },
            &Settings::default(),
            now,
        );
        let row = AlertRuleRow {
            trace_id: "run".into(),
            trace_ref: "scope".into(),
            start_ms: now.timestamp_millis(),
            duration_ms,
            duration_span_id: "latest".into(),
            error_span_id: String::new(),
        };
        assert_eq!(
            matches(&row, &settings, now)
                .iter()
                .map(|matched| matched.rule_id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }

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
        let settings = Settings::updated(
            AlertRuleConfig {
                revision: 1,
                rules: vec![
                    AlertRule {
                        id: "duration".into(),
                        enabled: duration_enabled,
                        condition: AlertRuleCondition::RunDuration { minutes: 5 },
                    },
                    AlertRule {
                        id: "errors".into(),
                        enabled: model_errors,
                        condition: AlertRuleCondition::ModelError,
                    },
                ],
            },
            &Settings::default(),
            now,
        );
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
