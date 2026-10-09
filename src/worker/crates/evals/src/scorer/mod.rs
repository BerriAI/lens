mod called_before;
mod judge;
mod task_completed;

pub use judge::{Judge, JudgeRequest};

use called_before::called_before;
use lens_contract::eval::{CalledBefore, Judge as JudgeScorer, Scorer};
use serde::{Deserialize, Serialize};
use task_completed::task_completed;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpanStatus {
    #[serde(alias = "STATUS_CODE_OK")]
    Ok,
    #[serde(alias = "STATUS_CODE_ERROR")]
    Error,
    #[serde(alias = "STATUS_CODE_UNSET")]
    Unset,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalSpan {
    pub span_id: String,
    pub parent_span_id: String,
    pub name: String,
    pub start_ns: i64,
    pub status: SpanStatus,
    #[serde(default)]
    pub tool_name: Option<String>,
}

pub(crate) async fn passes<J: Judge>(
    scorer: &Scorer,
    case_id: &str,
    trial: usize,
    spans: &[EvalSpan],
    judge: &J,
) -> Option<bool> {
    match scorer {
        Scorer::TaskCompleted(_) => Some(task_completed(spans)),
        Scorer::CalledBefore(CalledBefore { first, then }) => {
            Some(called_before(spans, first, then))
        }
        Scorer::Judge(JudgeScorer { prompt, model }) => {
            judge::passes(
                judge,
                JudgeRequest {
                    case_id,
                    trial,
                    prompt,
                    model,
                    spans,
                },
            )
            .await
        }
    }
}
