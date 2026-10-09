mod called_before;
mod judge;
mod task_completed;

pub use called_before::called_before;
pub use judge::{JUDGE_PASS_THRESHOLD, Judge, JudgeRequest};
pub use task_completed::{root, task_completed};

use lens_contract::eval::{CalledBefore, Judge as JudgeScorer, Scorer};
use serde::{Deserialize, Serialize};

use crate::Result;

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
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub output: String,
}

pub async fn passes<J: Judge>(
    scorer: &Scorer,
    case_id: &str,
    spans: &[EvalSpan],
    judge: &J,
) -> Result<bool> {
    match scorer {
        Scorer::TaskCompleted(_) => Ok(task_completed(spans)),
        Scorer::CalledBefore(CalledBefore { first, then }) => Ok(called_before(spans, first, then)),
        Scorer::Judge(JudgeScorer { prompt, model }) => {
            judge::passes(
                judge,
                JudgeRequest {
                    case_id,
                    prompt,
                    model,
                    spans,
                },
            )
            .await
        }
    }
}
