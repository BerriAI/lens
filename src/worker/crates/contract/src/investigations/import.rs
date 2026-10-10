use crate::worker::{
    EvidenceQuote, FindingDraftDescription, FindingDraftPriority, FindingDraftTitle,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FindingImport {
    pub fingerprint: String,
    pub agent_name: String,
    pub category: FindingCategory,
    pub title: FindingDraftTitle,
    pub description: FindingDraftDescription,
    pub suggestion: String,
    pub limitation: String,
    pub priority: FindingDraftPriority,
    pub evidence: Vec<FindingSource>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum FindingCategory {
    Performance,
    #[serde(rename = "Agent quality")]
    Quality,
    Reliability,
}

impl FindingCategory {
    pub fn check_id(self) -> &'static str {
        match self {
            Self::Performance => "performance",
            Self::Quality => "quality",
            Self::Reliability => "reliability",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FindingSource {
    pub trace_id: String,
    pub trace_ref: String,
    pub span_id: String,
    pub quote: EvidenceQuote,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FindingImported {
    pub lens_id: String,
    pub finding_id: String,
}
