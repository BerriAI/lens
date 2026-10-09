use super::*;

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["input","expected"]))]
pub struct AgentTestCase {
    pub expected: AgentTestCaseExpected,
    pub input: AgentTestCaseInput,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["check_id","title","hypothesis","execution_ids"]))]
pub struct Candidate {
    pub check_id: String,
    pub execution_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_finding_id: Option<String>,
    pub hypothesis: String,
    #[serde(default)]
    #[schemars(extend("default" = "issue"))]
    pub kind: CandidateKind,
    pub title: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct Clusters {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub candidates: Vec<Candidate>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["execution_id","span_id","quote"]))]
pub struct Evidence {
    pub execution_id: String,
    pub quote: EvidenceQuote,
    #[serde(default)]
    #[schemars(extend("default" = "support"))]
    pub role: EvidenceRole,
    pub span_id: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct Extraction {
    #[serde(default)]
    #[schemars(extend("default" = false))]
    pub cannot_assess: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub observations: Vec<Observation>,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub reasoning: ExtractionReasoning,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["title","description","check_id","evidence","id","first_seen","last_seen","revision"]))]
pub struct Finding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<IssueBrief>,
    pub check_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub check_ids: Vec<String>,
    pub description: FindingDescription,
    #[schemars(extend("minItems" = 1))]
    pub evidence: Vec<Evidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_finding_id: Option<String>,
    pub first_seen: DateTime<Utc>,
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub investigation_runs: Vec<String>,
    #[serde(default)]
    #[schemars(extend("default" = "issue"))]
    pub kind: FindingKind,
    pub last_seen: DateTime<Utc>,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub limitation: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub merged_finding_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub occurrences: Vec<String>,
    #[serde(default)]
    #[schemars(extend("default" = "medium"))]
    pub priority: FindingPriority,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub reason: String,
    pub revision: i64,
    #[serde(default)]
    #[schemars(extend("default" = "open"))]
    pub status: FindingStatus,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub suggestion: String,
    pub title: FindingTitle,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["title","description","check_id","evidence"]))]
pub struct FindingDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<IssueBrief>,
    pub check_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub check_ids: Vec<String>,
    pub description: FindingDraftDescription,
    #[schemars(extend("minItems" = 1))]
    pub evidence: Vec<Evidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_finding_id: Option<String>,
    #[serde(default)]
    #[schemars(extend("default" = "issue"))]
    pub kind: FindingDraftKind,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub limitation: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub merged_finding_ids: Vec<String>,
    #[serde(default)]
    #[schemars(extend("default" = "medium"))]
    pub priority: FindingDraftPriority,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub suggestion: String,
    pub title: FindingDraftTitle,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingGroup {
    #[schemars(extend("minItems" = 1))]
    pub members: Vec<String>,
    pub representative: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingGroups {
    pub groups: Vec<FindingGroup>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct Findings {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub findings: Vec<FindingDraft>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["problem","user_goal","what_happened","test_cases"]))]
pub struct IssueBrief {
    pub problem: IssueBriefProblem,
    #[schemars(extend("minItems" = 1))]
    pub test_cases: Vec<AgentTestCase>,
    pub user_goal: IssueBriefUserGoal,
    pub what_happened: IssueBriefWhatHappened,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub check_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    #[schemars(extend("default" = "issue"))]
    pub kind: ObservationKind,
    pub summary: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunAssessment {
    #[serde(default)]
    #[schemars(extend("default" = false))]
    pub cannot_assess: bool,
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub issue_checks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub pattern_checks: Vec<String>,
}
