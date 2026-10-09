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
    #[error("Lens HTTP {status}: {code}")]
    Api { status: u16, code: String },
    #[error("Could not reach Lens within the request deadline")]
    Transport(#[source] reqwest::Error),
    #[error("Lens returned an invalid response")]
    Response(#[source] serde_json::Error),
    #[error("{0}")]
    Infrastructure(&'static str),
    #[error("Could not access the setup file")]
    Io(#[from] std::io::Error),
    #[error("Invalid TOML configuration")]
    Toml(#[from] toml::de::Error),
}

impl Error {
    pub fn hint(&self) -> &'static str {
        match self {
            Self::Api { status: 401, .. } => "Set LENS_API_KEY to a valid Lens credential",
            Self::Api { status: 403, .. } => {
                "Use a key allowed to access Lens routes; inference-only keys are insufficient"
            }
            Self::Api { code, .. } if code == "revision_not_found" => {
                "Choose an existing dataset revision"
            }
            Self::Api { code, .. } if code == "dataset_not_found" => {
                "Check the dataset name and your access"
            }
            Self::Api { code, .. } if code == "contract_version" => {
                "The SDK and server use different contract versions"
            }
            Self::Api { status: 404, code } if code == "request_failed" => {
                "Check LENS_BASE_URL and whether the eval API is deployed"
            }
            _ => "",
        }
    }
}
