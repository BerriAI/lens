use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

pub type JudgeError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("An eval run needs at least one case and one trial per case")]
    EmptyRun,
    #[error("Case {case_id} has {received} trials, more than the {expected} expected")]
    ExtraTrials {
        case_id: String,
        received: usize,
        expected: usize,
    },
    #[error("Case {case_id} appears more than once in the run")]
    DuplicateCase { case_id: String },
    #[error("The judge could not score the trial")]
    Judge(#[source] JudgeError),
    #[error("The judge returned {score}, outside 0..=1")]
    JudgeScore { score: f64 },
}
