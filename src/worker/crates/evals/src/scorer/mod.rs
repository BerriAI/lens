mod called_before;
mod judge;
mod task_completed;

pub use called_before::called_before;
pub use judge::{JUDGE_PASS_THRESHOLD, Judge, JudgeRequest};
pub use task_completed::{root, task_completed};

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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Scorer {
    TaskCompleted,
    CalledBefore {
        first: String,
        then: String,
    },
    Judge {
        prompt: String,
        #[serde(default)]
        model: String,
    },
}

impl Scorer {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::TaskCompleted => "task_completed",
            Self::CalledBefore { .. } => "called_before",
            Self::Judge { .. } => "judge",
        }
    }

    pub async fn passes<J: Judge>(
        &self,
        case_id: &str,
        spans: &[EvalSpan],
        judge: &J,
    ) -> Result<bool> {
        match self {
            Self::TaskCompleted => Ok(task_completed(spans)),
            Self::CalledBefore { first, then } => Ok(called_before(spans, first, then)),
            Self::Judge { prompt, model } => {
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
}

pub fn scorer_keys(scorers: &[Scorer]) -> Vec<String> {
    scorers
        .iter()
        .enumerate()
        .map(|(index, scorer)| {
            let kind = scorer.kind();
            let same = |other: &&Scorer| other.kind() == kind;
            match scorers.iter().filter(same).count() {
                1 => kind.to_owned(),
                _ => format!(
                    "{kind}_{}",
                    scorers[..index].iter().filter(same).count() + 1
                ),
            }
        })
        .collect()
}
