#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Stored signal data is invalid")]
    Json(#[from] serde_json::Error),
    #[error("Decisions response is invalid")]
    Response(#[source] serde_json::Error),
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Decisions(#[from] DecisionsError),
}

#[derive(Debug, thiserror::Error)]
#[error("Lens signal trace content is unavailable")]
pub struct SourceError(#[source] pub Box<dyn std::error::Error + Send + Sync>);

#[derive(Debug, thiserror::Error)]
pub enum DecisionsError {
    #[error("Lens signal provider returned HTTP {status}")]
    Provider { status: u16 },
    #[error("Lens signal model request timed out")]
    Timeout,
    #[error("Lens signal provider response exceeds the configured limit")]
    ResponseLimit,
    #[error("Lens signal model request failed")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("Lens signal storage is unavailable")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
}
