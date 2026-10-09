#![forbid(unsafe_code)]

mod baseline;
mod error;
mod gate;
pub mod scorer;
mod summary;
mod verdict;

pub use baseline::Baseline;
pub use error::{Error, JudgeError, Result};
pub use gate::{GateFacts, check_gate};
pub use scorer::{EvalSpan, Judge, JudgeRequest, SpanStatus};
pub use summary::{CaseInput, Evaluation, RunInput, evaluate};
pub use verdict::{Trial, TrialOutcome, majority};
