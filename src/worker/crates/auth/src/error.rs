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
    #[error("The gateway is set in the Lens environment and cannot be changed here")]
    GatewayLocked,
    #[error("Enter the gateway address as an http or https URL")]
    InvalidGateway,
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

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error(
        "This MCP connection already exists. Open Access to manage it or create a new connection"
    )]
    Exists,
    #[error("You cannot manage this MCP connection")]
    Forbidden,
    #[error("Invalid MCP connection credential")]
    Unauthorized,
    #[error("MCP connection expired. Create a replacement connection")]
    Expired,
    #[error("MCP connection revoked. Create a replacement connection")]
    Revoked,
    #[error("MCP connection not found")]
    NotFound,
    #[error("MCP credential generation is unavailable")]
    Random(#[source] rand::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}
