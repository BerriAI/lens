#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Each check must have a unique ID")]
    DuplicateCheck,
    #[error("Describe expected behavior or add an enabled check")]
    MissingChecks,
    #[error("expected_behavior is reserved for the behavior description")]
    ReservedCheck,
    #[error("Sample percentage must be finite, greater than zero and at most 100")]
    SamplePercentage,
    #[error("Monthly budget must be finite and greater than zero")]
    MonthlyBudget,
    #[error("{0} exceeds the supported calendar range")]
    CalendarRange(&'static str),
    #[error("Choose both a start and an end time")]
    IncompleteWindow,
    #[error("Start time must be before end time")]
    WindowOrder,
    #[error("Agent name must have at most 200 characters")]
    AgentName,
    #[error("Review count exceeds the supported range")]
    ReviewCount,
    #[error("Investigation revision exceeds the supported range")]
    RevisionCount,
    #[error(transparent)]
    Conversion(#[from] lens_contract::ConversionError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("Lens state changed; retry the operation")]
    Conflict,
    #[error("Lens storage is unavailable")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("This worker no longer owns the job")]
    Ownership,
    #[error(transparent)]
    Invalid(#[from] Error),
    #[error(transparent)]
    Store(#[from] RepositoryError),
}
