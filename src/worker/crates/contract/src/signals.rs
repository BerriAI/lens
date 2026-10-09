use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, de::Error};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Signal {
    #[serde(deserialize_with = "signal_id")]
    #[schemars(regex(pattern = "^[a-z][a-z0-9_]{0,63}$"))]
    pub id: String,
    #[serde(deserialize_with = "signal_name")]
    #[schemars(length(min = 1, max = 60))]
    pub name: String,
    #[serde(deserialize_with = "signal_question")]
    #[schemars(length(min = 3, max = 500))]
    pub question: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignalConfig {
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_threshold", deserialize_with = "threshold")]
    #[schemars(range(min = 0.05, max = 0.95))]
    pub threshold: f64,
    #[serde(default = "default_signals", deserialize_with = "signals")]
    #[schemars(length(max = 20))]
    pub signals: Vec<Signal>,
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            threshold: default_threshold(),
            signals: default_signals(),
        }
    }
}

fn default_threshold() -> f64 {
    0.5
}

fn default_signals() -> Vec<Signal> {
    [
        ("user_frustration", "User frustration", "Does the user show frustration, annoyance or dissatisfaction with the agent in this run, for example complaints, irritated corrections, all caps, profanity, or giving up on the task?"),
        ("missing_capability", "Missing capability", "Does the user ask for something the agent cannot do in this run, so that the agent refuses, says it lacks a tool, permission, integration or data source, or fails because the capability does not exist?"),
        ("repeated_request", "Repeated request", "Does the user ask for the same thing more than once in this run, usually because the agent did not deliver it the first time?"),
    ]
    .into_iter()
    .map(|(id, name, question)| Signal { id: id.into(), name: name.into(), question: question.into() })
    .collect()
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignalFlag {
    pub signal_id: String,
    pub name: String,
    #[serde(deserialize_with = "score")]
    #[schemars(range(min = 0, max = 1))]
    pub score: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NoulAnswer {
    Noul {
        #[serde(deserialize_with = "score")]
        #[schemars(range(min = 0, max = 1))]
        noul: f64,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TraceSignalStatus {
    Unclassified,
    Pending,
    Classified,
    Failed,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceSignals {
    pub trace_id: String,
    #[serde(default)]
    pub trace_ref: String,
    pub status: TraceSignalStatus,
    #[serde(default)]
    pub flags: Vec<SignalFlag>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub classified_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignalStep {
    pub kind: String,
    pub name: String,
    pub content: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignalStatus {
    #[default]
    Pending,
    Classified,
    Failed,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, JsonSchema)]
pub struct SignalData {
    #[serde(default)]
    pub status: SignalStatus,
    #[serde(default, deserialize_with = "scores")]
    pub scores: BTreeMap<String, f64>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignalAttemptStatus {
    Classified,
    Failed,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignalAttempt {
    pub status: SignalAttemptStatus,
    #[serde(default, deserialize_with = "scores")]
    pub scores: BTreeMap<String, f64>,
    pub model: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredTraceSignal {
    pub trace_id: String,
    #[serde(default)]
    pub trace_ref: String,
    pub config_key: String,
    pub span_count: i64,
    #[serde(default, deserialize_with = "timestamp")]
    pub claimed_until: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "timestamp")]
    pub classified_at: Option<DateTime<Utc>>,
    pub data: Value,
}

fn signal_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let id = String::deserialize(deserializer)?;
    if id.len() > 64
        || !id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(D::Error::custom("Invalid signal ID"));
    }
    Ok(id)
}

fn bounded_string<'de, D: Deserializer<'de>>(
    deserializer: D,
    min: usize,
    max: usize,
) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    if !(min..=max).contains(&value.chars().count()) {
        return Err(D::Error::custom(
            "Signal text length is outside the allowed range",
        ));
    }
    Ok(value)
}

fn signal_name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    bounded_string(deserializer, 1, 60)
}

fn signal_question<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    bounded_string(deserializer, 3, 500)
}

fn probability(value: Value, min: f64, max: f64) -> Option<f64> {
    let number = value
        .as_f64()
        .or_else(|| value.as_bool().map(|flag| u8::from(flag) as f64))
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))?;
    (number.is_finite() && (min..=max).contains(&number)).then_some(number)
}

fn threshold<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    probability(Value::deserialize(deserializer)?, 0.05, 0.95).ok_or_else(|| {
        D::Error::custom("Signal threshold must be finite and between 0.05 and 0.95")
    })
}

fn score<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    probability(Value::deserialize(deserializer)?, 0.0, 1.0)
        .ok_or_else(|| D::Error::custom("Signal score must be finite and between 0 and 1"))
}

fn scores<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<String, f64>, D::Error> {
    BTreeMap::<String, Value>::deserialize(deserializer)?
        .into_iter()
        .map(|(key, value)| {
            probability(value, 0.0, 1.0)
                .map(|score| (key, score))
                .ok_or_else(|| D::Error::custom("Signal score must be finite and between 0 and 1"))
        })
        .collect()
}

fn signals<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Signal>, D::Error> {
    let signals = Vec::<Signal>::deserialize(deserializer)?;
    if signals.len() > 20 {
        return Err(D::Error::custom("A maximum of 20 signals is allowed"));
    }
    if signals
        .iter()
        .map(|signal| &signal.id)
        .collect::<BTreeSet<_>>()
        .len()
        != signals.len()
    {
        return Err(D::Error::custom("Signal IDs must be unique"));
    }
    Ok(signals)
}

fn timestamp<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<DateTime<Utc>>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .map(|value| {
            DateTime::parse_from_rfc3339(&value)
                .map(|value| value.with_timezone(&Utc))
                .or_else(|_| {
                    NaiveDateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M:%S%.f")
                        .map(|value| value.and_utc())
                })
                .or_else(|_| {
                    NaiveDateTime::parse_from_str(&value, "%Y-%m-%d %H:%M:%S%.f")
                        .map(|value| value.and_utc())
                })
                .map_err(D::Error::custom)
        })
        .transpose()
}
