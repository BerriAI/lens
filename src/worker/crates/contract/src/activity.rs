use crate::worker::{LensSettings, LensSettingsSource, MetadataFilter};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ActivitySelection {
    pub source: LensSettingsSource,
    pub service: String,
    pub agent_name: String,
    pub filters: Vec<MetadataFilter>,
    pub sample_size: Option<NonZeroU64>,
    pub sample_percent: f64,
    pub team_id: String,
    pub execution_ids: Vec<String>,
}

impl Default for ActivitySelection {
    fn default() -> Self {
        Self {
            source: LensSettingsSource::Traces,
            service: String::new(),
            agent_name: String::new(),
            filters: Vec::new(),
            sample_size: None,
            sample_percent: 100.0,
            team_id: String::new(),
            execution_ids: Vec::new(),
        }
    }
}

impl From<&LensSettings> for ActivitySelection {
    fn from(settings: &LensSettings) -> Self {
        Self {
            source: settings.source,
            service: settings.service.clone(),
            agent_name: settings.agent_name.clone(),
            filters: settings.filters.clone(),
            sample_size: settings.sample_size,
            sample_percent: settings.sample_percent,
            team_id: settings.team_id.clone(),
            execution_ids: settings.execution_ids.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Preview {
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    #[serde(default)]
    pub offset: u64,
    pub selection: ActivitySelection,
    #[serde(default = "lookback")]
    pub lookback_hours: NonZeroU64,
}

fn lookback() -> NonZeroU64 {
    NonZeroU64::new(24).unwrap()
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActivityAvailability {
    pub traces: bool,
    pub requests: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AnalysisModelInfo {
    pub model_group: String,
    pub providers: Vec<String>,
    pub mode: String,
    pub supported_openai_params: Option<Vec<String>>,
}
