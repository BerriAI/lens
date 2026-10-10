use litellm_storage_clickhouse::Query;
use litellm_traces::query::named as contracts;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(remote = "contracts::TraceConversationAnchor")]
struct AnchorEncoding {
    pub trace_ref: String,
    pub team_id: String,
    pub api_key_hash: String,
    pub session_id: String,
    #[serde(deserialize_with = "super::number::deserialize")]
    pub start_ns: i64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AnchorRow(#[serde(with = "AnchorEncoding")] pub contracts::TraceConversationAnchor);

pub struct ConversationAnchor;

impl Query for ConversationAnchor {
    type Params = contracts::TraceConversationAnchorParams;
    type Row = AnchorRow;
    const SQL: &'static str = include_str!("../../query/trace_conversation_anchor.sql");
}

#[derive(Deserialize, Serialize)]
#[serde(remote = "contracts::TraceConversationRow")]
struct TurnEncoding {
    pub trace_id: String,
    pub trace_ref: String,
    pub span_id: String,
    #[serde(deserialize_with = "super::number::deserialize")]
    pub start_ns: i64,
    pub input: String,
    pub output: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TurnRow(#[serde(with = "TurnEncoding")] pub contracts::TraceConversationRow);

pub struct ConversationTurns;

impl Query for ConversationTurns {
    type Params = contracts::TraceConversationTurnsParams;
    type Row = TurnRow;
    const SQL: &'static str = include_str!("../../query/trace_conversation_turns.sql");
}
