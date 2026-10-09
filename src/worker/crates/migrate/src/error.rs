#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid migration command or missing configuration: {0}")]
    Configuration(&'static str),
    #[error("A source record does not match its stored identity or supported schema")]
    InvalidRecord,
    #[error("The source contains a duplicate target identity")]
    Duplicate,
    #[error("The target contains existing state or belongs to a different migration")]
    TargetConflict,
    #[error("The imported state did not match the source snapshot")]
    Verification,
    #[error("Could not read the PostgreSQL snapshot")]
    Postgres(#[from] sqlx::Error),
    #[error("Could not decode the source metadata")]
    Json(#[from] serde_json::Error),
    #[error("Could not access ClickHouse state")]
    Storage(#[from] litellm_storage_clickhouse::Error),
    #[error("Could not initialize the ClickHouse indexes")]
    Indexes(#[from] lens_investigations::RepositoryError),
    #[error("Could not configure the HTTP client")]
    Client(#[from] litellm_http::Error),
}
