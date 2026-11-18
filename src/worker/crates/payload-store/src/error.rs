#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Configuration(&'static str),
    #[error("payload object key is invalid")]
    InvalidKey,
    #[error("payload store request failed")]
    Request(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("payload object is missing")]
    Missing,
    #[error("payload object does not match its recorded length or checksum")]
    Integrity,
}
