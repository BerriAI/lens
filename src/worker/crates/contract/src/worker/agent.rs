use super::*;

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub working_notes: CheckpointWorkingNotes,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceReply {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub catalog: Vec<CatalogEntry>,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub error: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub parts: Vec<TracePart>,
    pub request: EvidenceRequest,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub review_catalog: Vec<ReviewIndex>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub reviews: Vec<ReviewRecord>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRequest {
    pub action: EvidenceRequestAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub char_end: Option<u64>,
    #[serde(default)]
    #[schemars(extend("minimum" = 0, "default" = 0))]
    pub char_start: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    #[serde(default)]
    #[schemars(extend("default" = false))]
    pub include_initial: bool,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_phase: Option<EvidenceRequestReviewPhase>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub span_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_end: Option<u64>,
    #[serde(default)]
    #[schemars(extend("minimum" = 0, "default" = 0))]
    pub turn_start: u64,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["role","content"]))]
pub struct ModelMessage {
    pub content: String,
    pub role: ModelMessageRole,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelRequest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub messages: Vec<ModelMessage>,
    pub prompt: ModelRequestPrompt,
    pub purpose: ModelRequestPurpose,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelResult {
    pub content: String,
    #[serde(default)]
    #[schemars(extend("default" = false))]
    pub context_exceeded: bool,
    pub cost: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<ModelResultFinishReason>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema, Default)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "PythonAgentTurn[Extraction]")]
pub struct PythonAgentTurnExtraction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<PythonAgentTurnExtractionCheckpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Extraction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub tools: Vec<PythonAgentTurnExtractionToolsItem>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(untagged)]
#[schemars(inline)]
pub enum PythonAgentTurnExtractionToolsItem {
    EvidenceRequest(EvidenceRequest),
    PythonRequest(PythonRequest),
}
impl ::std::convert::From<EvidenceRequest> for PythonAgentTurnExtractionToolsItem {
    fn from(value: EvidenceRequest) -> Self {
        Self::EvidenceRequest(value)
    }
}
impl ::std::convert::From<PythonRequest> for PythonAgentTurnExtractionToolsItem {
    fn from(value: PythonRequest) -> Self {
        Self::PythonRequest(value)
    }
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema, Default)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "PythonAgentTurn[Findings]")]
pub struct PythonAgentTurnFindings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<PythonAgentTurnFindingsCheckpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Findings>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub tools: Vec<PythonAgentTurnFindingsToolsItem>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(untagged)]
#[schemars(inline)]
pub enum PythonAgentTurnFindingsToolsItem {
    EvidenceRequest(EvidenceRequest),
    PythonRequest(PythonRequest),
}
impl ::std::convert::From<EvidenceRequest> for PythonAgentTurnFindingsToolsItem {
    fn from(value: EvidenceRequest) -> Self {
        Self::EvidenceRequest(value)
    }
}
impl ::std::convert::From<PythonRequest> for PythonAgentTurnFindingsToolsItem {
    fn from(value: PythonRequest) -> Self {
        Self::PythonRequest(value)
    }
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PythonRequest {
    #[schemars(extend("const" = "python"))]
    pub action: String,
    pub code: PythonRequestCode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub execution_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub span_ids: Vec<String>,
}
