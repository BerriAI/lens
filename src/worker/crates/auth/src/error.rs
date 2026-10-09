#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Lens state changed; retry the operation")]
    Conflict,
    #[error("Lens storage is unavailable")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Unauthorized(&'static str),
    #[error("Lens session requests must come from the Lens origin")]
    OriginMismatch,
    #[error("invalid authentication configuration: {0}")]
    Configuration(&'static str),
    #[error(transparent)]
    Store(#[from] StoreError),
}
