use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    feedback::TraceIdentity,
    signals::{
        SignalConfig, SignalData, SignalFlag, SignalStatus, StoredTraceSignal, TraceSignalStatus,
        TraceSignals,
    },
    worker::Execution,
};
use sha2::{Digest, Sha256};

use crate::Error;

pub fn config_key(config: &SignalConfig) -> String {
    let payload = serde_json::json!({"model":config.model,"signals":config.signals.iter().map(|signal| serde_json::json!({"id":signal.id,"question":signal.question})).collect::<Vec<_>>()});
    format!("{:x}", Sha256::digest(payload.to_string().as_bytes()))
}

pub fn enabled(config: &SignalConfig) -> bool {
    !config.model.is_empty() && !config.signals.is_empty()
}

pub fn candidate(
    trace: &Execution,
    existing: Option<&StoredTraceSignal>,
    config_key: &str,
    now: DateTime<Utc>,
) -> bool {
    let Some(existing) = existing else {
        return true;
    };
    if existing.claimed_until.is_some_and(|lease| lease > now) {
        return false;
    }
    if existing.config_key != config_key {
        return true;
    }
    let status = existing
        .data
        .get("status")
        .and_then(serde_json::Value::as_str);
    if status == Some("pending") {
        return existing.claimed_until.is_some();
    }
    if existing.span_count > trace.span_count {
        return false;
    }
    if existing.span_count < trace.span_count {
        return existing
            .classified_at
            .is_some_and(|time| time < now - TimeDelta::minutes(5));
    }
    status == Some("failed")
        && existing
            .classified_at
            .is_some_and(|time| time < now - TimeDelta::minutes(30))
}

pub fn claimable(
    trace: &Execution,
    existing: Option<&StoredTraceSignal>,
    config_key: &str,
    now: DateTime<Utc>,
) -> bool {
    let Some(existing) = existing else {
        return true;
    };
    if existing.claimed_until.is_some_and(|lease| lease >= now) {
        return false;
    }
    if existing.config_key != config_key {
        return true;
    }
    let status = existing
        .data
        .get("status")
        .and_then(serde_json::Value::as_str);
    let expired_pending = status == Some("pending") && existing.claimed_until.is_some();
    let grew = trace.span_count > existing.span_count
        && existing
            .classified_at
            .is_some_and(|time| time < now - TimeDelta::minutes(5));
    let retry = status == Some("failed")
        && existing
            .classified_at
            .is_some_and(|time| time < now - TimeDelta::minutes(30));
    expired_pending || grew || retry
}

pub fn trace_signals(
    trace: &TraceIdentity,
    existing: Option<&StoredTraceSignal>,
    config: &SignalConfig,
) -> Result<TraceSignals, Error> {
    let mut result = TraceSignals {
        trace_id: trace.trace_id.clone(),
        trace_ref: trace.trace_ref.clone(),
        status: TraceSignalStatus::Unclassified,
        flags: Vec::new(),
        model: String::new(),
        classified_at: None,
    };
    let Some(existing) = existing.filter(|row| row.config_key == config_key(config)) else {
        return Ok(result);
    };
    let object: serde_json::Map<String, serde_json::Value> =
        serde_json::from_value(existing.data.clone())?;
    let data: SignalData = serde_json::from_value(serde_json::Value::Object(object))?;
    result.model = data.model;
    if data.status == SignalStatus::Pending {
        result.status = TraceSignalStatus::Pending;
        return Ok(result);
    }
    result.classified_at = existing.classified_at;
    if data.status == SignalStatus::Failed || !data.error.is_empty() {
        result.status = TraceSignalStatus::Failed;
        return Ok(result);
    }
    result.status = TraceSignalStatus::Classified;
    result.flags = config
        .signals
        .iter()
        .filter_map(|signal| {
            data.scores
                .get(&signal.id)
                .filter(|score| **score >= config.threshold)
                .map(|score| SignalFlag {
                    signal_id: signal.id.clone(),
                    name: signal.name.clone(),
                    score: *score,
                })
        })
        .collect();
    result
        .flags
        .sort_by(|left, right| right.score.total_cmp(&left.score));
    Ok(result)
}
