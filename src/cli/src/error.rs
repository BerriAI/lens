use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Configuration(&'static str),
    #[error("{message}: {value}")]
    Invalid {
        message: &'static str,
        value: String,
    },
    #[error("Could not access the setup file")]
    Io(#[from] std::io::Error),
    #[error("Could not read the tasks CSV")]
    Tasks(#[from] csv::Error),
    #[error("Could not read or write the run checkpoint")]
    Checkpoint(#[source] serde_json::Error),
    #[error(transparent)]
    Sdk(#[from] lens_evals_sdk::Error),
}
