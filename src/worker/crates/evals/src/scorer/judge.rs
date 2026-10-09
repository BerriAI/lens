use std::future::Future;

use super::EvalSpan;
use crate::{Error, JudgeError, Result};

pub const JUDGE_PASS_THRESHOLD: f64 = 0.5;

#[derive(Clone, Copy, Debug)]
pub struct JudgeRequest<'a> {
    pub case_id: &'a str,
    pub prompt: &'a str,
    pub model: &'a str,
    pub spans: &'a [EvalSpan],
    pub output: Option<&'a str>,
}

pub trait Judge: Sync {
    fn score(
        &self,
        request: JudgeRequest<'_>,
    ) -> impl Future<Output = std::result::Result<f64, JudgeError>> + Send;
}

pub(super) async fn passes<J: Judge>(judge: &J, request: JudgeRequest<'_>) -> Result<bool> {
    let score = judge.score(request).await.map_err(Error::Judge)?;
    if !(0.0..=1.0).contains(&score) {
        return Err(Error::JudgeScore { score });
    }
    Ok(score >= JUDGE_PASS_THRESHOLD)
}
