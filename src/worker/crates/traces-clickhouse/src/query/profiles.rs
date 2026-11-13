use litellm_storage_clickhouse::Query;

pub struct ProfileRuns;

#[macro_rules_attribute::apply(crate::wire_type)]
#[derive(Debug)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(deny_unknown_fields))]
pub struct ProfileRunsParams {
    #[serde(
        deserialize_with = "super::number::boolean",
        serialize_with = "litellm_traces::wire::serialize_flag"
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(schema_with = "litellm_traces::schema::flag")
    )]
    pub all_teams: bool,
    pub user_id: String,
    pub team_ids: Vec<String>,
    pub agent: String,
    #[serde(deserialize_with = "super::number::deserialize")]
    pub start_ms: i64,
    #[serde(deserialize_with = "super::number::deserialize")]
    pub end_ms: i64,
    #[serde(deserialize_with = "super::number::deserialize")]
    pub limit: u32,
}

#[macro_rules_attribute::apply(crate::wire_type)]
#[derive(Debug)]
#[cfg_attr(feature = "schema", schemars(rename = "ProfileRunRow"))]
pub struct ProfileRunsRow {
    pub team_id: String,
    pub api_key_hash: String,
    pub trace_id: String,
    #[serde(deserialize_with = "super::number::deserialize")]
    pub start_ms: i64,
    #[serde(deserialize_with = "super::number::deserialize")]
    pub end_ms: i64,
    pub profile_id: String,
    pub category: String,
    pub input_preview: String,
    #[serde(deserialize_with = "super::number::deserialize")]
    #[cfg_attr(
        feature = "schema",
        schemars(schema_with = "crate::wire_schema::u64_number")
    )]
    pub agent_count: u64,
}

impl Query for ProfileRuns {
    type Params = ProfileRunsParams;
    type Row = ProfileRunsRow;

    const SQL: &'static str = concat!(
        include_str!("../../query/profile_runs_head.sql"),
        include_str!("../../query/trace_profiles.sql"),
        include_str!("../../query/profile_runs.sql")
    );
}

pub struct CallCostModels;

#[macro_rules_attribute::apply(crate::wire_type)]
#[derive(Debug)]
#[serde(deny_unknown_fields)]
pub struct CallCostModelsParams {
    #[serde(
        deserialize_with = "super::number::boolean",
        serialize_with = "litellm_traces::wire::serialize_flag"
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(schema_with = "litellm_traces::schema::flag")
    )]
    pub all_teams: bool,
    pub user_id: String,
    pub team_ids: Vec<String>,
    pub request_ids: Vec<String>,
    #[serde(deserialize_with = "super::number::deserialize")]
    pub start_ms: i64,
    #[serde(deserialize_with = "super::number::deserialize")]
    pub end_ms: i64,
}

#[macro_rules_attribute::apply(crate::wire_type)]
#[derive(Debug)]
pub struct CallCostModelsRow {
    pub team_id: String,
    pub request_id: String,
    #[serde(deserialize_with = "super::number::deserialize")]
    pub start_ms: i64,
    pub model_id: String,
    pub model_group: String,
    pub model: String,
    pub provider: String,
}

impl Query for CallCostModels {
    type Params = CallCostModelsParams;
    type Row = CallCostModelsRow;

    const SQL: &'static str = include_str!("../../query/call_cost_models.sql");
}
