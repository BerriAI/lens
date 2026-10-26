use lens_contract::worker::{Candidate, Findings};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnalysisCheckpoint {
    Grouping {
        candidates: Vec<Candidate>,
        batches: i64,
    },
    Checking {
        result: Findings,
    },
}
