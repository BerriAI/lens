use super::*;

pub type CatalogSpan = (String, String, String, String, Option<i64>, String, String);

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["execution","spans","partial","characters"]))]
pub struct CatalogEntry {
    pub characters: Option<i64>,
    pub execution: Execution,
    pub partial: bool,
    pub spans: Vec<CatalogSpan>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["id","source","trace_id","team_id","name","start_time","span_count"]))]
pub struct Execution {
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub metadata: Vec<MetadataFilter>,
    pub name: String,
    #[serde(default)]
    #[schemars(extend("default" = false))]
    pub root_seen: bool,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub service: String,
    pub source: ExecutionSource,
    pub span_count: i64,
    pub start_time: String,
    pub team_id: String,
    pub trace_id: String,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub trace_ref: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionContent {
    pub execution: Execution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default)]
    #[schemars(extend("default" = false))]
    pub partial: bool,
    pub parts: Vec<TracePart>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["execution_id","trace_id","agent","name","model","duration_ms","at"]))]
pub struct Review {
    pub agent: String,
    pub at: DateTime<Utc>,
    #[serde(default)]
    #[schemars(extend("default" = false))]
    pub cannot_assess: bool,
    #[serde(default)]
    #[schemars(extend("default" = false))]
    pub consolidated: bool,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub content_version: String,
    #[schemars(extend("minimum" = 0))]
    pub duration_ms: u64,
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction: Option<Extraction>,
    pub model: String,
    pub name: String,
    #[serde(default)]
    #[schemars(extend("default" = false))]
    pub partial: bool,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub reasoning: ReviewReasoning,
    #[serde(default)]
    #[schemars(extend("default" = false))]
    pub reused: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("maxItems" = 8, "default" = []))]
    pub spans: Vec<ReviewSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub tool_calls: Vec<ToolCount>,
    pub trace_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub verdicts: Vec<ReviewVerdict>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["execution_id","phase","characters"]))]
pub struct ReviewIndex {
    pub characters: i64,
    pub execution_id: String,
    pub phase: ReviewIndexPhase,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["execution_id","phase","content"]))]
pub struct ReviewRecord {
    pub content: String,
    pub execution_id: String,
    pub phase: ReviewRecordPhase,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["span_id","name","kind","preview"]))]
pub struct ReviewSpan {
    #[serde(default)]
    #[schemars(extend("default" = false))]
    pub cited: bool,
    pub kind: ReviewSpanKind,
    pub name: ReviewSpanName,
    pub preview: ReviewSpanPreview,
    pub span_id: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewVerdict {
    pub check_id: String,
    pub kind: ReviewVerdictKind,
    pub summary: ReviewVerdictSummary,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["execution_id","content_version"]))]
pub struct ReviewVersion {
    pub content_version: String,
    pub execution_id: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["execution_id","span_id","name","kind","content"]))]
pub struct TracePart {
    pub content: String,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub end_time: String,
    pub execution_id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub parent_span_id: String,
    pub span_id: String,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub start_time: String,
    #[serde(default)]
    #[schemars(extend("default" = false))]
    pub truncated: bool,
}
