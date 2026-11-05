#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("task embedding model unavailable")]
    Model(#[source] fastembed::Error),
    #[error("task clusters exceed the supported limit or contain invalid embeddings")]
    InvalidClusters,
    #[error("task embedding model returned an invalid embedding")]
    InvalidEmbedding,
}
