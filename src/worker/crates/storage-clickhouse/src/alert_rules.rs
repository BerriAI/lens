use chrono::{DateTime, Utc};
use lens_contract::alert_rules::{AlertRuleConfig, AlertRuleKind, AlertRuleMatch};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    Error, signal_alerts,
    state::{Change, ClickHouseState},
};

pub const SETTINGS: &str = "alert-rules/settings";
pub const MATCHES: &str = "alert-rules/matches/";

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Settings {
    pub config: AlertRuleConfig,
    pub duration_since: Option<DateTime<Utc>>,
    pub errors_since: Option<DateTime<Utc>>,
}

impl Settings {
    pub fn enabled(&self, kind: AlertRuleKind) -> bool {
        match kind {
            AlertRuleKind::RunDuration => self.config.run_duration_minutes.is_some(),
            AlertRuleKind::ModelError => self.config.model_errors,
        }
    }

    pub fn since(&self, kind: AlertRuleKind) -> Option<DateTime<Utc>> {
        match kind {
            AlertRuleKind::RunDuration => self.duration_since,
            AlertRuleKind::ModelError => self.errors_since,
        }
    }
}

pub async fn settings(store: &ClickHouseState) -> Result<Settings, Error> {
    let snapshot = store.read(SETTINGS).await?;
    let mut settings: Settings = if snapshot.value.is_null() {
        Settings::default()
    } else {
        serde_json::from_value(snapshot.value).map_err(|_| Error::InvalidResponse)?
    };
    settings.config.revision = snapshot.head.revision;
    Ok(settings)
}

pub async fn record(
    store: &ClickHouseState,
    matched: &AlertRuleMatch,
    started_at: DateTime<Utc>,
) -> Result<(), Error> {
    let digest = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(&matched.trace_id, &matched.trace_ref, matched.kind.id()))
                .map_err(|_| Error::InvalidState)?
        )
    );
    let previous = store.read(&format!("{MATCHES}{digest}")).await?;
    if !previous.value.is_null() {
        return Ok(());
    }
    let mut changes = vec![Change {
        previous,
        value: serde_json::to_value(matched).map_err(|_| Error::InvalidState)?,
    }];
    let notifications = signal_alerts::settings(store).await?;
    if notifications.enabled
        && notifications
            .activated_at
            .is_some_and(|at| started_at >= at)
    {
        let previous = store
            .read(&format!("{}{digest}", signal_alerts::EVENTS))
            .await?;
        if previous.value.is_null() {
            let event = signal_alerts::Event {
                generation: notifications.generation,
                signal_id: matched.kind.id().into(),
                name: matched.kind.name().into(),
                score: 0.0,
                trace_id: matched.trace_id.clone(),
                trace_ref: matched.trace_ref.clone(),
                created_at: matched.detected_at,
                status: signal_alerts::DeliveryStatus::Pending,
                attempts: 0,
                retry_at: matched.detected_at,
                rule: Some(matched.clone()),
            };
            changes.push(Change {
                previous,
                value: serde_json::to_value(event).map_err(|_| Error::InvalidState)?,
            });
        }
    }
    match store.commit(changes).await {
        Err(Error::StateConflict) => Ok(()),
        result => result,
    }
}

pub async fn recent(
    store: &ClickHouseState,
    since: DateTime<Utc>,
    limit: u32,
) -> Result<Vec<AlertRuleMatch>, Error> {
    let body = store
        .command(
            "SELECT b.data AS data FROM lens_state_blobs AS b \
         INNER JOIN lens_state_heads AS h USING (key, revision, digest) \
         WHERE startsWith(b.key, {prefix:String}) \
         AND parseDateTime64BestEffortOrNull(JSONExtractString(b.data, 'detected_at'), 6, 'UTC') \
             >= parseDateTime64BestEffort({since:String}, 6, 'UTC') \
         GROUP BY b.key, b.data \
         ORDER BY JSONExtractString(b.data, 'detected_at') DESC, b.key \
         LIMIT {limit:UInt32} FORMAT JSONEachRow",
            &[
                ("prefix", MATCHES.into()),
                ("since", since.to_rfc3339()),
                ("limit", limit.to_string()),
            ],
            String::new(),
        )
        .await?;
    #[derive(Deserialize)]
    struct Row {
        data: String,
    }
    body.lines()
        .map(|line| {
            let row: Row = serde_json::from_str(line).map_err(|_| Error::InvalidResponse)?;
            serde_json::from_str(&row.data).map_err(|_| Error::InvalidResponse)
        })
        .collect()
}
