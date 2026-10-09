use crate::Error;
use litellm_http::{
    Client, ClientVariant, HttpClientPool, HttpSettings, Resolution, media::PublicDnsResolver,
};
use litellm_traces_clickhouse::Config as StorageConfig;
use std::{net::SocketAddr, sync::Arc, time::Duration};

pub struct Config {
    pub address: SocketAddr,
    pub proxy_url: url::Url,
    pub worker_token: String,
    pub service_token: String,
    pub release: String,
    pub storage: StorageConfig,
    pub authentication: Option<lens_auth::Settings>,
    pub datasets: lens_server::datasets::DatasetConfig,
}

fn required(name: &'static str) -> Result<String, Error> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or(Error::Configuration(name))
}

impl Config {
    pub fn from_env() -> Result<Self, Error> {
        let proxy_url = url::Url::parse(&required("LITELLM_URL")?)
            .map_err(|_| Error::Configuration("LITELLM_URL"))?;
        if !matches!(proxy_url.scheme(), "http" | "https")
            || !proxy_url.username().is_empty()
            || proxy_url.password().is_some()
            || proxy_url.query().is_some()
            || proxy_url.fragment().is_some()
        {
            return Err(Error::Configuration("LITELLM_URL"));
        }
        let service_token = required("LITELLM_LENS_SERVICE_TOKEN")?;
        let worker_token = std::env::var("LENS_WORKER_TOKEN")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| service_token.clone());
        if service_token.len() < 32 {
            return Err(Error::Configuration(
                "LITELLM_LENS_SERVICE_TOKEN must contain at least 32 characters",
            ));
        }
        Ok(Self {
            datasets: dataset_config(|name| std::env::var(name).ok()),
            authentication: std::env::var("LENS_ADMIN_TOKEN")
                .ok()
                .map(|token| {
                    lens_auth::Settings::new(
                        &token,
                        std::env::var("LENS_GATEWAY_SECRET")
                            .ok()
                            .filter(|value| !value.is_empty()),
                        &std::env::var("LENS_PUBLIC_URL")
                            .unwrap_or_else(|_| "http://localhost:4000".into()),
                    )
                })
                .transpose()?,
            address: std::env::var("LITELLM_LENS_LISTEN")
                .unwrap_or_else(|_| "0.0.0.0:4318".into())
                .parse()
                .map_err(|_| Error::Configuration("LITELLM_LENS_LISTEN"))?,
            proxy_url,
            worker_token,
            service_token,
            release: required("LITELLM_RELEASE_TAG")?,
            storage: StorageConfig::new(
                std::env::var("CLICKHOUSE_DATABASE").unwrap_or_else(|_| "litellm".into()),
                &clickhouse_url()?,
                std::env::var("AGENT_TRACING_RETENTION_DAYS")
                    .unwrap_or_else(|_| "14".into())
                    .parse()
                    .map_err(|_| Error::Configuration("AGENT_TRACING_RETENTION_DAYS"))?,
                65_536,
            )?,
        })
    }
}

fn dataset_config(read: impl Fn(&str) -> Option<String>) -> lens_server::datasets::DatasetConfig {
    lens_server::datasets::DatasetConfig {
        limits: lens_datasets::Limits {
            max_cases: integer_or_default(read("LENS_DATASET_MAX_CASES").as_deref(), 200),
            max_case_chars: integer_or_default(
                read("LENS_DATASET_MAX_CASE_CHARS").as_deref(),
                20_000,
            ),
        },
        trace_retry_after_seconds: integer_or_default(
            read("TRACE_READ_RETRY_AFTER_SECONDS").as_deref(),
            2,
        ),
    }
}

fn integer_or_default(raw: Option<&str>, default: i64) -> i64 {
    raw.and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

fn clickhouse_url() -> Result<String, Error> {
    if let Ok(url) = required("CLICKHOUSE_URL") {
        return Ok(url);
    }
    let mut url = url::Url::parse("http://localhost:8123")
        .map_err(|_| Error::Configuration("CLICKHOUSE_HOST"))?;
    url.set_host(Some(&required("CLICKHOUSE_HOST")?))
        .map_err(|_| Error::Configuration("CLICKHOUSE_HOST"))?;
    url.set_username(&std::env::var("CLICKHOUSE_USER").unwrap_or_else(|_| "default".into()))
        .map_err(|_| Error::Configuration("CLICKHOUSE_USER"))?;
    url.set_password(Some(&required("CLICKHOUSE_PASSWORD")?))
        .map_err(|_| Error::Configuration("CLICKHOUSE_PASSWORD"))?;
    Ok(url.into())
}

pub fn http_client() -> Result<Client, Error> {
    let settings = HttpSettings {
        connect_timeout: Duration::from_secs(5),
        ..HttpSettings::default()
    };
    Ok(HttpClientPool::new(Arc::new(PublicDnsResolver)).client(
        &Resolution::from(&settings).config,
        ClientVariant::NoRedirect,
    )?)
}

#[cfg(test)]
mod tests {
    use super::{dataset_config, integer_or_default};
    use rstest::rstest;

    #[rstest]
    #[case::absent(None, 200)]
    #[case::empty(Some(""), 200)]
    #[case::invalid(Some("invalid"), 200)]
    #[case::whitespace(Some(" \t42\n"), 42)]
    #[case::zero(Some("0"), 0)]
    #[case::negative(Some("-1"), -1)]
    fn dataset_limits_keep_configured_values_and_fallbacks(
        #[case] raw: Option<&str>,
        #[case] expected: i64,
    ) {
        assert_eq!(integer_or_default(raw, 200), expected);
    }

    #[rstest]
    fn dataset_configuration_reads_each_supported_setting() {
        let config = dataset_config(|name| match name {
            "LENS_DATASET_MAX_CASES" => Some("17".into()),
            "LENS_DATASET_MAX_CASE_CHARS" => Some("900".into()),
            "TRACE_READ_RETRY_AFTER_SECONDS" => Some("7".into()),
            _ => panic!("unexpected configuration setting"),
        });
        assert_eq!(config.limits.max_cases, 17);
        assert_eq!(config.limits.max_case_chars, 900);
        assert_eq!(config.trace_retry_after_seconds, 7);
    }

    #[rstest]
    fn missing_dataset_configuration_preserves_defaults() {
        let config = dataset_config(|_| None);
        assert_eq!(config.limits.max_cases, 200);
        assert_eq!(config.limits.max_case_chars, 20_000);
        assert_eq!(config.trace_retry_after_seconds, 2);
    }
}
