#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    #[error("shorter than {0} characters")]
    TooShort(usize),
    #[error("longer than {0} characters")]
    TooLong(usize),
    #[error("invalid value")]
    InvalidValue,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidAgentIo {
    #[error("agent_io.version must be 1")]
    Version,
    #[error(
        "agent_io.connection must be a local profile name of 1 to 64 ASCII letters, digits, underscores or hyphens, starting with a letter"
    )]
    Connection,
    #[error(
        "agent_io.submit.path must be a relative origin path without query, fragment or template syntax"
    )]
    RequestPath,
    #[error("agent_io.submit.accepted_status must be between 200 and 299")]
    AcceptedStatus,
    #[error("agent_io.input must include a case.input binding")]
    MissingInput,
    #[error(
        "agent_io.input targets must select existing scalar or empty-array leaves in submit.json"
    )]
    InputTarget,
    #[error("agent_io.input targets must not duplicate or overlap")]
    OverlappingInputTargets,
    #[error("agent_io JSON pointers must use RFC 6901 syntax")]
    JsonPointer,
    #[error(
        "agent_io.completion.path_template must be a relative path containing exactly one {{id}} placeholder"
    )]
    PollPath,
    #[error("agent_io.completion success and failure states must be nonempty, unique and disjoint")]
    PollStates,
    #[error(
        "agent_io.completion requires interval_ms between 250 and 60000, timeout_ms between 1000 and 3600000, and interval_ms no greater than timeout_ms"
    )]
    PollTiming,
    #[error("agent_io.completion.require_item.matches must not be empty")]
    EmptyMatch,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidCaseResult {
    #[error("case result requires output or trace, or an exclusive error")]
    NeedsOutputOrTraceOrError,
    #[error("case result trace value must not be empty")]
    EmptyTraceValue,
    #[error("case result error message is too long")]
    ErrorMessageTooLong,
    #[error("case result cost must be finite and nonnegative")]
    NegativeOrNonFiniteCost,
}
