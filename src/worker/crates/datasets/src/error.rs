#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error(transparent)]
    Storage(#[from] StoreError),
    #[error("Lens not found")]
    LensNotFound,
    #[error("{0}")]
    InvalidRequest(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("{0}")]
    TraceChanged(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("Trace is too large for this view. Use a filtered trace query.")]
    TooLarge,
    #[error("Traces are temporarily unavailable. Please try again.")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Lens state changed; retry the operation")]
    Conflict,
    #[error("Lens storage is unavailable")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RevisionProblem {
    #[error("A dataset holds at most {0} cases")]
    TooManyCases(i64),
    #[error("Each case must be at most {0} characters")]
    CaseTooLarge(i64),
}
