#![forbid(unsafe_code)]

mod baseline;
mod error;
mod gate;
pub mod scorer;
mod summary;
mod verdict;

pub use baseline::{Baseline, CaseDiff};
pub use error::{Error, JudgeError, Result};
pub use gate::{Gate, GateFacts, GateResult, check_gate};
pub use scorer::{EvalSpan, Judge, JudgeRequest, Scorer, SpanStatus, scorer_keys};
pub use summary::{CaseInput, Evaluation, RunInput, Summary, evaluate};
pub use verdict::{Trial, TrialOutcome, majority};
