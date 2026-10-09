use super::*;

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Check {
    #[serde(default = "defaults::default_bool::<true>")]
    #[schemars(extend("default" = true))]
    pub enabled: bool,
    pub id: CheckId,
    pub instruction: CheckInstruction,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["lens_id","job","findings"]))]
pub struct Claim {
    pub findings: Vec<Finding>,
    pub job: Job,
    pub lens_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviews: Option<Vec<Review>>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["id","created_at","start","end","settings","revision"]))]
pub struct Job {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub activities: Vec<Activity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub assessments: Vec<RunAssessment>,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub attempts: i64,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub cost: f64,
    #[serde(default)]
    #[schemars(extend("default" = {"candidates":0,"eligible":0,"failed_tasks":0,"grouped_batches":0,"grouping_batches":0,"inconclusive":0,"investigated":0,"partial":0,"reusable":0,"reused":0,"screened":0,"selected":0,"unassessable":0}))]
    pub coverage: Coverage,
    pub created_at: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub findings: Option<Vec<Finding>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_until: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub reading: Vec<InFlight>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub review_versions: Vec<ReviewVersion>,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub reviewed: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub reviews: Vec<Review>,
    pub revision: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<Sample>,
    pub settings: LensSettings,
    #[serde(default = "defaults::job_stage")]
    #[schemars(extend("default" = "Queued"))]
    pub stage: String,
    pub start: DateTime<Utc>,
    #[serde(default)]
    #[schemars(extend("default" = "queued"))]
    pub status: JobStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub steps: Vec<Step>,
    #[serde(default)]
    #[schemars(extend("default" = "schedule"))]
    pub trigger: JobTrigger,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["name","model"]))]
pub struct LensSettings {
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub agent_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub checks: Vec<Check>,
    #[serde(default = "defaults::default_nzu64::<8>")]
    #[schemars(extend("minimum" = 1, "default" = 8))]
    pub concurrency: NonZeroU64,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub context: String,
    #[serde(default = "defaults::default_bool::<true>")]
    #[schemars(extend("default" = true))]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub execution_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub filters: Vec<MetadataFilter>,
    #[serde(default = "defaults::default_nzu64::<15>")]
    #[schemars(extend("minimum" = 1, "default" = 15))]
    pub interval_minutes: NonZeroU64,
    #[serde(default = "defaults::default_nzu64::<24>")]
    #[schemars(extend("minimum" = 1, "default" = 24))]
    pub lookback_hours: NonZeroU64,
    pub model: LensSettingsModel,
    #[serde(default = "defaults::lens_settings_monthly_budget")]
    #[schemars(extend("exclusiveMinimum" = 0, "default" = 100))]
    pub monthly_budget: f64,
    pub name: LensSettingsName,
    #[serde(default = "defaults::lens_settings_sample_percent")]
    #[schemars(extend("maximum" = 100, "exclusiveMinimum" = 0, "default" = 100))]
    pub sample_percent: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_size: Option<NonZeroU64>,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub service: String,
    #[serde(default)]
    #[schemars(extend("default" = "traces"))]
    pub source: LensSettingsSource,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub team_id: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MetadataFilter {
    pub key: MetadataFilterKey,
    pub value: MetadataFilterValue,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Result {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub assessments: Vec<RunAssessment>,
    pub coverage: Coverage,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub error: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub findings: Vec<FindingDraft>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub review_versions: Vec<ReviewVersion>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["executions","eligible"]))]
pub struct Sample {
    pub eligible: i64,
    pub executions: Vec<Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<i64>,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub selected: i64,
}
