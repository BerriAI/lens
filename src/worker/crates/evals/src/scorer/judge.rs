use std::future::Future;

use super::EvalSpan;
use crate::JudgeError;

const JUDGE_PASS_THRESHOLD: f64 = 0.5;

#[derive(Clone, Copy, Debug)]
pub struct JudgeRequest<'a> {
    pub case_id: &'a str,
    pub trial: usize,
    pub prompt: &'a str,
    pub model: &'a str,
    pub spans: &'a [EvalSpan],
}

pub trait Judge: Sync {
    fn score(
        &self,
        request: JudgeRequest<'_>,
    ) -> impl Future<Output = std::result::Result<f64, JudgeError>> + Send;
}

pub(super) async fn passes<J: Judge>(judge: &J, request: JudgeRequest<'_>) -> Option<bool> {
    let score = judge.score(request).await.ok()?;
    (0.0..=1.0)
        .contains(&score)
        .then_some(score >= JUDGE_PASS_THRESHOLD)
}
