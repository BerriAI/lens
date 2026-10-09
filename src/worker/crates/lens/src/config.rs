use crate::Error;
use litellm_http::{
    Client, ClientVariant, HttpClientPool, HttpSettings, Resolution, media::PublicDnsResolver,
};
use litellm_traces_clickhouse::Config as StorageConfig;
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

pub struct Config {
    pub address: SocketAddr,
    pub mode: Mode,
    pub storage: StorageConfig,
    pub authentication: Option<lens_auth::Settings>,
    pub datasets: lens_server::datasets::DatasetConfig,
    pub traces: lens_server::tracing::TraceConfig,
    pub ui_directory: Option<PathBuf>,
    pub query_secret: String,
    pub ingestion_url: String,
    pub release: String,
    pub public_url: String,
    pub eval_judge_api_key: Option<String>,
    pub eval_judge_model: Option<String>,
}

pub enum Mode {
    Standalone,
    Gateway(Gateway),
}

pub struct Gateway {
    pub proxy_url: url::Url,
    pub worker_token: String,
    pub service_token: String,
    pub release: String,
}

fn required(read: &impl Fn(&str) -> Option<String>, name: &'static str) -> Result<String, Error> {
    read(name)
        .filter(|value| !value.is_empty())
        .ok_or(Error::Configuration(name))
}

impl Config {
    pub fn from_env() -> Result<Self, Error> {
        Self::read(|name| std::env::var(name).ok())
    }

    fn read(read: impl Fn(&str) -> Option<String>) -> Result<Self, Error> {
        let mode = match read("LENS_MODE").as_deref() {
            Some("standalone") => Mode::Standalone,
            Some("gateway") => Mode::Gateway(Gateway::read(&read)?),
            None if read("LITELLM_URL").is_some() => Mode::Gateway(Gateway::read(&read)?),
            None => Mode::Standalone,
            _ => {
                return Err(Error::Configuration(
                    "LENS_MODE must be standalone or gateway",
                ));
            }
        };
        let (admin_token, query_secret, database, public_url) = match &mode {
            Mode::Standalone => {
                let token = required(&read, "LENS_ADMIN_TOKEN")?;
                (Some(token.clone()), token, "lens", "http://localhost:4318")
            }
            Mode::Gateway(gateway) => (
                read("LENS_ADMIN_TOKEN"),
                gateway.service_token.clone(),
                "litellm",
                "http://localhost:4000",
            ),
        };
        let public_url = read("LENS_PUBLIC_URL").unwrap_or_else(|| public_url.into());
        let ingestion_url = read("LITELLM_LENS_PUBLIC_URL").unwrap_or_else(|| public_url.clone());
        let parsed = url::Url::parse(&ingestion_url)
            .map_err(|_| Error::Configuration("LITELLM_LENS_PUBLIC_URL"))?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || !parsed.path().trim_matches('/').is_empty()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(Error::Configuration("LITELLM_LENS_PUBLIC_URL"));
        }
        let release = match &mode {
            Mode::Standalone => read("LENS_VERSION")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").into()),
            Mode::Gateway(gateway) => gateway.release.clone(),
        };
        Ok(Self {
            public_url: public_url.clone(),
            eval_judge_api_key: read("LITELLM_API_KEY"),
            eval_judge_model: read("LENS_EVAL_JUDGE_MODEL"),
            datasets: dataset_config(&read),
            traces: trace_config(&read)?,
            authentication: admin_token
                .map(|token| {
                    lens_auth::Settings::new(
                        &token,
                        read("LENS_GATEWAY_SECRET").filter(|value| !value.is_empty()),
                        &public_url,
                    )
                })
                .transpose()?,
            address: read("LITELLM_LENS_LISTEN")
                .unwrap_or_else(|| "0.0.0.0:4318".into())
                .parse()
                .map_err(|_| Error::Configuration("LITELLM_LENS_LISTEN"))?,
            ui_directory: read("LENS_UI_DIRECTORY")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            query_secret,
            ingestion_url: ingestion_url.trim_end_matches('/').into(),
            release,
            storage: StorageConfig::new(
                read("CLICKHOUSE_DATABASE").unwrap_or_else(|| database.into()),
                &clickhouse_url(&read, matches!(mode, Mode::Standalone))?,
                read("AGENT_TRACING_RETENTION_DAYS")
                    .unwrap_or_else(|| "14".into())
                    .parse()
                    .map_err(|_| Error::Configuration("AGENT_TRACING_RETENTION_DAYS"))?,
                65_536,
            )?,
            mode,
        })
    }
}

impl Gateway {
    fn read(read: &impl Fn(&str) -> Option<String>) -> Result<Self, Error> {
        let proxy_url = url::Url::parse(&required(read, "LITELLM_URL")?)
            .map_err(|_| Error::Configuration("LITELLM_URL"))?;
        if !matches!(proxy_url.scheme(), "http" | "https")
            || !proxy_url.username().is_empty()
            || proxy_url.password().is_some()
            || proxy_url.query().is_some()
            || proxy_url.fragment().is_some()
        {
            return Err(Error::Configuration("LITELLM_URL"));
        }
        let service_token = required(read, "LITELLM_LENS_SERVICE_TOKEN")?;
        let worker_token = read("LENS_WORKER_TOKEN")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| service_token.clone());
        if service_token.len() < 32 {
            return Err(Error::Configuration(
                "LITELLM_LENS_SERVICE_TOKEN must contain at least 32 characters",
            ));
        }
        Ok(Self {
            proxy_url,
            worker_token,
            service_token,
            release: required(read, "LITELLM_RELEASE_TAG")?,
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

fn trace_config(
    read: &impl Fn(&str) -> Option<String>,
) -> Result<lens_server::tracing::TraceConfig, Error> {
    Ok(lens_server::tracing::TraceConfig {
        list_limit: integer_or_default(read("AGENT_TRACING_LIST_PAGE_SIZE").as_deref(), 50)
            .try_into()
            .map_err(|_| Error::Configuration("AGENT_TRACING_LIST_PAGE_SIZE"))?,
        agent_limit: integer_or_default(read("AGENT_TRACING_AGENT_LIST_LIMIT").as_deref(), 500)
            .try_into()
            .map_err(|_| Error::Configuration("AGENT_TRACING_AGENT_LIST_LIMIT"))?,
        retention_days: integer_or_default(read("AGENT_TRACING_RETENTION_DAYS").as_deref(), 14),
        retry_after_seconds: integer_or_default(
            read("TRACE_READ_RETRY_AFTER_SECONDS").as_deref(),
            2,
        ),
    })
}

fn clickhouse_url(
    read: &impl Fn(&str) -> Option<String>,
    standalone: bool,
) -> Result<String, Error> {
    if let Ok(url) = required(read, "CLICKHOUSE_URL") {
        return Ok(url);
    }
    if standalone && read("CLICKHOUSE_HOST").is_none() {
        return Ok("http://localhost:8123".into());
    }
    let mut url = url::Url::parse("http://localhost:8123")
        .map_err(|_| Error::Configuration("CLICKHOUSE_HOST"))?;
    url.set_host(Some(&required(read, "CLICKHOUSE_HOST")?))
        .map_err(|_| Error::Configuration("CLICKHOUSE_HOST"))?;
    url.set_username(&read("CLICKHOUSE_USER").unwrap_or_else(|| "default".into()))
        .map_err(|_| Error::Configuration("CLICKHOUSE_USER"))?;
    url.set_password(Some(&required(read, "CLICKHOUSE_PASSWORD")?))
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
    use super::{Config, Mode, dataset_config, integer_or_default};
    use crate::Error;
    use rstest::{fixture, rstest};
    use std::collections::BTreeMap;

    #[fixture]
    fn standalone() -> BTreeMap<&'static str, &'static str> {
        BTreeMap::from([(
            "LENS_ADMIN_TOKEN",
            "standalone-setup-token-at-least-32-characters",
        )])
    }

    #[fixture]
    fn gateway() -> BTreeMap<&'static str, &'static str> {
        BTreeMap::from([
            ("LITELLM_URL", "http://localhost:4000/gateway"),
            (
                "LITELLM_LENS_SERVICE_TOKEN",
                "gateway-service-token-at-least-32-characters",
            ),
            ("LITELLM_RELEASE_TAG", "test-release"),
            ("CLICKHOUSE_URL", "http://localhost:8123"),
        ])
    }

    #[rstest]
    fn standalone_requires_no_gateway_settings(standalone: BTreeMap<&str, &str>) {
        let config =
            Config::read(|key| standalone.get(key).map(|value| value.to_string())).unwrap();
        assert!(matches!(config.mode, Mode::Standalone));
        assert!(config.authentication.is_some());
        assert_eq!(config.storage.storage().database(), "lens");
        assert_eq!(config.address.to_string(), "0.0.0.0:4318");
        assert_eq!(config.query_secret, standalone["LENS_ADMIN_TOKEN"]);
        assert_eq!(config.ingestion_url, "http://localhost:4318");
        assert_eq!(config.release, env!("CARGO_PKG_VERSION"));
    }

    #[rstest]
    #[case::missing(None)]
    #[case::empty(Some(""))]
    #[case::short(Some("short"))]
    fn standalone_never_starts_without_strong_authentication(#[case] token: Option<&str>) {
        let result = Config::read(|key| {
            (key == "LENS_ADMIN_TOKEN")
                .then_some(token)
                .flatten()
                .map(str::to_owned)
        });
        assert!(matches!(
            result,
            Err(Error::Configuration("LENS_ADMIN_TOKEN") | Error::Authentication(_))
        ));
    }

    #[rstest]
    #[case::legacy(None)]
    #[case::explicit(Some("gateway"))]
    fn gateway_configuration_preserves_existing_worker_contract(
        gateway: BTreeMap<&str, &str>,
        #[case] mode: Option<&str>,
    ) {
        let config = Config::read(|key| {
            if key == "LENS_MODE" {
                mode
            } else {
                gateway.get(key).copied()
            }
            .map(str::to_owned)
        })
        .unwrap();
        let Mode::Gateway(worker) = config.mode else {
            panic!("expected gateway mode")
        };
        assert_eq!(worker.proxy_url.as_str(), gateway["LITELLM_URL"]);
        assert_eq!(worker.worker_token, gateway["LITELLM_LENS_SERVICE_TOKEN"]);
        assert_eq!(worker.service_token, gateway["LITELLM_LENS_SERVICE_TOKEN"]);
        assert_eq!(worker.release, gateway["LITELLM_RELEASE_TAG"]);
        assert_eq!(config.query_secret, worker.service_token);
        assert_eq!(config.storage.storage().database(), "litellm");
        assert!(config.authentication.is_none());
    }

    #[rstest]
    fn explicit_standalone_ignores_legacy_worker_variables(standalone: BTreeMap<&str, &str>) {
        let config = Config::read(|key| match key {
            "LENS_MODE" => Some("standalone".into()),
            "LITELLM_URL" => Some("invalid-gateway-url".into()),
            _ => standalone.get(key).map(|value| value.to_string()),
        })
        .unwrap();
        assert!(matches!(config.mode, Mode::Standalone));
        assert!(config.authentication.is_some());
    }

    #[rstest]
    #[case::unknown("other")]
    #[case::empty("")]
    fn invalid_mode_is_rejected(#[case] mode: &str) {
        let result = Config::read(|key| (key == "LENS_MODE").then(|| mode.into()));
        assert!(matches!(
            result,
            Err(Error::Configuration(
                "LENS_MODE must be standalone or gateway"
            ))
        ));
    }

    #[rstest]
    #[case::url("LITELLM_URL")]
    #[case::token("LITELLM_LENS_SERVICE_TOKEN")]
    #[case::release("LITELLM_RELEASE_TAG")]
    fn explicit_gateway_still_requires_its_settings(
        gateway: BTreeMap<&str, &str>,
        #[case] missing: &str,
    ) {
        let result = Config::read(|key| match key {
            "LENS_MODE" => Some("gateway".into()),
            key if key == missing => None,
            key => gateway.get(key).map(|value| value.to_string()),
        });
        assert!(matches!(result, Err(Error::Configuration(name)) if name == missing));
    }

    #[rstest]
    fn standalone_uses_configured_storage_and_ui(standalone: BTreeMap<&str, &str>) {
        let config = Config::read(|key| match key {
            "CLICKHOUSE_DATABASE" => Some("lens_custom".into()),
            "LITELLM_LENS_LISTEN" => Some("127.0.0.1:4100".into()),
            "LENS_UI_DIRECTORY" => Some("/app/ui".into()),
            _ => standalone.get(key).map(|value| value.to_string()),
        })
        .unwrap();
        assert_eq!(config.storage.storage().database(), "lens_custom");
        assert_eq!(config.address.to_string(), "127.0.0.1:4100");
        assert_eq!(config.ui_directory.unwrap().to_str(), Some("/app/ui"));
    }

    #[rstest]
    #[case::public_origin(None, "https://lens.example.test")]
    #[case::separate_ingestion(Some("https://traces.example.test/"), "https://traces.example.test")]
    fn standalone_advertises_the_configured_ingestion_origin(
        standalone: BTreeMap<&str, &str>,
        #[case] ingestion: Option<&str>,
        #[case] expected: &str,
    ) {
        let config = Config::read(|key| match key {
            "LENS_PUBLIC_URL" => Some("https://lens.example.test".into()),
            "LITELLM_LENS_PUBLIC_URL" => ingestion.map(str::to_owned),
            "LENS_VERSION" => Some("test-candidate".into()),
            _ => standalone.get(key).map(|value| value.to_string()),
        })
        .unwrap();
        assert_eq!(config.ingestion_url, expected);
        assert_eq!(config.release, "test-candidate");
        assert!(config.authentication.unwrap().secure_cookie());
    }

    #[rstest]
    #[case::scheme("file:///tmp/traces")]
    #[case::credentials("https://user:password@lens.example.test")]
    #[case::path("https://lens.example.test/path")]
    #[case::query("https://lens.example.test?token=secret")]
    #[case::fragment("https://lens.example.test#fragment")]
    fn invalid_ingestion_origins_are_rejected(
        standalone: BTreeMap<&str, &str>,
        #[case] origin: &str,
    ) {
        let result = Config::read(|key| {
            if key == "LITELLM_LENS_PUBLIC_URL" {
                Some(origin.into())
            } else {
                standalone.get(key).map(|value| value.to_string())
            }
        });
        assert!(matches!(
            result,
            Err(Error::Configuration("LITELLM_LENS_PUBLIC_URL"))
        ));
    }

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
