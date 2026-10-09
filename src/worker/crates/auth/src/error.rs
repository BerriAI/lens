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

#[derive(Debug, thiserror::Error)]
pub enum IngestionError {
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("{field} must contain {min} to {max} characters")]
    InvalidLength {
        field: &'static str,
        min: usize,
        max: usize,
    },
    #[error("Choose an expiry in the future")]
    InvalidExpiry,
    #[error("Lens ingestion key already exists")]
    AlreadyExists,
    #[error("Revoke an unused ingestion key before creating another")]
    KeyLimit,
    #[error("Lens ingestion key limit exceeded")]
    CatalogLimit,
    #[error("Lens credential generation is unavailable")]
    Random(#[source] rand::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}
