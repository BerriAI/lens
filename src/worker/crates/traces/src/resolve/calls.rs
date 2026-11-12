use super::{resolution::Resolution, spend::request_cost};
use crate::{
    SpendMatch,
    query::named::{SpendByResponseIdsRow as SpendRow, TraceSpansRow},
};

/// One leaf model call in a run, with the gateway ledger rows that strict matching tied to it.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelCall {
    pub span_id: String,
    pub agent: String,
    pub model: String,
    pub duration_ns: u64,
    pub state: Option<SpendMatch>,
    pub requests: Vec<LedgerRequest>,
    pub spend: Option<f64>,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LedgerRequest {
    pub team_id: String,
    pub start_ms: i64,
    pub request_id: String,
}

pub fn model_calls(rows: &[TraceSpansRow], spend: &[SpendRow]) -> Vec<ModelCall> {
    if rows.is_empty() {
        return Vec::new();
    }
    let resolution = Resolution::new(rows, spend);
    resolution
        .model_calls
        .iter()
        .map(|call| {
            let row = resolution.row(*call);
            let matched = resolution.call_match(*call);
            let requests = matched
                .and_then(|matched| matched.requests.clone())
                .unwrap_or_default();
            ModelCall {
                span_id: row.span_id.clone(),
                agent: resolution.owner(*call).to_owned(),
                model: row.model.clone(),
                duration_ns: row.duration_ns,
                state: matched.map(|matched| matched.state),
                spend: if requests.is_empty() {
                    None
                } else {
                    request_cost(&requests)
                },
                requests: requests
                    .iter()
                    .map(|request| LedgerRequest {
                        team_id: request.team_id.clone(),
                        start_ms: request.start_ms,
                        request_id: request.request_id.clone(),
                    })
                    .collect(),
            }
        })
        .collect()
}
