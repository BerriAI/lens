use crate::Error;
use litellm_http::{
    Client, ClientVariant, HttpClientPool, HttpSettings, Resolution, media::PublicDnsResolver,
};
use litellm_traces_clickhouse::Config as StorageConfig;
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

mod analysis;
mod evaluation;

pub struct Config {
    pub address: SocketAddr,
    pub storage: StorageConfig,
    pub authentication: lens_auth::Settings,
    pub datasets: lens_server::datasets::DatasetConfig,
    pub traces: lens_server::tracing::TraceConfig,
    pub ui_directory: Option<PathBuf>,
    pub query_secret: String,
    pub ingestion_url: String,
    pub public_url: url::Url,
    pub signal_alerts_ui_url: url::Url,
    pub release: String,
    pub analysis_models: Vec<lens_analysis::Deployment>,
    pub evaluation_models: Vec<lens_decisions::Deployment>,
    pub gateway_inference: Option<lens_inference::GatewayBase>,
    pub gateway_models: Option<crate::gateway::GatewayConfig>,
    pub gateway_service_token: Option<String>,
    pub eval_judge_api_key: Option<String>,
    pub eval_judge_model: Option<String>,
    pub github: Option<lens_server::github::GitHubApp>,
    pub github_service_url: Option<url::Url>,
    pub github_service_enabled: bool,
    pub slack: Option<crate::slack::SlackConfig>,
}

fn required(read: &impl Fn(&str) -> Option<String>, name: &'static str) -> Result<String, Error> {
    read(name)
        .filter(|value| !value.is_empty())
        .ok_or(Error::Configuration(name))
}

pub fn sign_in_link(
    read: impl Fn(&str) -> Option<String>,
    public_url: &url::Url,
    admin_token: &str,
) -> Option<url::Url> {
    let local = match public_url.host()? {
        url::Host::Domain(domain) => domain == "localhost",
        url::Host::Ipv4(address) => address.is_loopback(),
        url::Host::Ipv6(address) => address.is_loopback(),
    };
    if !local || read("LENS_SIGN_IN_LINK").as_deref() != Some("true") {
        return None;
    }
    let mut link: url::Url = format!("{}/ui/", public_url.as_str().trim_end_matches('/'))
        .parse()
        .ok()?;
    let token: String = url::form_urlencoded::byte_serialize(admin_token.as_bytes()).collect();
    link.set_fragment(Some(&format!("token={token}")));
    Some(link)
}

impl Config {
    pub fn from_env() -> Result<Self, Error> {
        Self::read(|name| std::env::var(name).ok())
    }

    pub(crate) fn read(read: impl Fn(&str) -> Option<String>) -> Result<Self, Error> {
        match read("LENS_MODE").as_deref() {
            None | Some("standalone") => {}
            _ => {
                return Err(Error::Configuration(
                    "Lens runs as a standalone service; remove LENS_MODE",
                ));
            }
        }
        let admin_token = required(&read, "LENS_ADMIN_TOKEN")?;
        let public_url = read("LENS_PUBLIC_URL").unwrap_or_else(|| "http://localhost:4318".into());
        let parsed_public_url =
            url::Url::parse(&public_url).map_err(|_| Error::Configuration("LENS_PUBLIC_URL"))?;
        let signal_alerts_ui_url = match read("LENS_SIGNAL_ALERTS_UI_URL") {
            Some(value) if !value.trim().is_empty() => url::Url::parse(value.trim())
                .ok()
                .filter(|url| {
                    matches!(url.scheme(), "http" | "https")
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none()
                })
                .ok_or(Error::Configuration("LENS_SIGNAL_ALERTS_UI_URL"))?,
            _ => parsed_public_url
                .join("/ui/")
                .map_err(|_| Error::Configuration("LENS_PUBLIC_URL"))?,
        };
        let ingestion_url = read("LITELLM_LENS_PUBLIC_URL").unwrap_or_else(|| public_url.clone());
        let parsed = url::Url::parse(&ingestion_url)
            .map_err(|_| Error::Configuration("LITELLM_LENS_PUBLIC_URL"))?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(Error::Configuration("LITELLM_LENS_PUBLIC_URL"));
        }
        let release = read("LENS_VERSION")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").into());
        let gateway_service_token = read("LITELLM_LENS_SERVICE_TOKEN");
        if gateway_service_token
            .as_ref()
            .is_some_and(|token| !(32..=512).contains(&token.len()))
        {
            return Err(Error::Configuration(
                "LITELLM_LENS_SERVICE_TOKEN must contain between 32 and 512 characters",
            ));
        }
        let authentication = lens_auth::Settings::new(
            &admin_token,
            read("LENS_GATEWAY_SECRET").filter(|value| !value.is_empty()),
            &public_url,
        )?;
        let authentication = match [
            "LENS_GATEWAY_PUBLIC_URL",
            "LENS_GATEWAY_URL",
            "LENS_GATEWAY_API_BASE",
        ]
        .into_iter()
        .find_map(|name| Some((name, read(name).filter(|value| !value.trim().is_empty())?)))
        {
            Some((name, url)) => authentication
                .with_gateway_login(url.trim())
                .map_err(|_| Error::Configuration(name))?,
            None => authentication,
        };
        let authentication = match read("LENS_SLACK_TEAM_ID") {
            Some(team)
                if read("LENS_SLACK_CHAT_ENABLED").as_deref() == Some("true")
                    && !team.trim().is_empty() =>
            {
                authentication.with_slack_investigator(
                    required(&read, "LENS_SLACK_LENS_API_KEY")?.trim(),
                    team.trim(),
                )?
            }
            _ => authentication,
        };
        Ok(Self {
            slack: crate::slack::SlackConfig::read(&read, &public_url)?,
            github_service_url: read("LENS_GITHUB_SERVICE_URL")
                .filter(|value| !value.is_empty())
                .map(|value| {
                    value
                        .parse()
                        .map_err(|_| Error::Configuration("LENS_GITHUB_SERVICE_URL"))
                })
                .transpose()?,
            github_service_enabled: match read("LENS_GITHUB_SERVICE_ENABLED").as_deref() {
                None | Some("") | Some("false") => false,
                Some("true") => true,
                Some(_) => return Err(Error::Configuration("LENS_GITHUB_SERVICE_ENABLED")),
            },
            github: github(&read, &public_url)?,
            gateway_service_token,
            eval_judge_api_key: read("LITELLM_API_KEY"),
            eval_judge_model: read("LENS_EVAL_JUDGE_MODEL"),
            analysis_models: analysis::read(&read)?,
            evaluation_models: evaluation::read(&read)?,
            gateway_inference: gateway_inference(&read)?,
            gateway_models: gateway_models(&read)?,
            datasets: dataset_config(&read),
            traces: trace_config(&read)?,
            authentication,
            address: read("LITELLM_LENS_LISTEN")
                .unwrap_or_else(|| "0.0.0.0:4318".into())
                .parse()
                .map_err(|_| Error::Configuration("LITELLM_LENS_LISTEN"))?,
            ui_directory: read("LENS_UI_DIRECTORY")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            query_secret: admin_token,
            ingestion_url: ingestion_url.trim_end_matches('/').into(),
            public_url: parsed_public_url,
            signal_alerts_ui_url,
            release,
            storage: StorageConfig::new(
                read("CLICKHOUSE_DATABASE").unwrap_or_else(|| "lens".into()),
                &clickhouse_url(&read)?,
                read("AGENT_TRACING_RETENTION_DAYS")
                    .unwrap_or_else(|| "14".into())
                    .parse()
                    .map_err(|_| Error::Configuration("AGENT_TRACING_RETENTION_DAYS"))?,
            )?,
        })
    }
}

fn github(
    read: &impl Fn(&str) -> Option<String>,
    public_url: &str,
) -> Result<Option<lens_server::github::GitHubApp>, Error> {
    let names = [
        "LENS_GITHUB_APP_SLUG",
        "LENS_GITHUB_CLIENT_ID",
        "LENS_GITHUB_CLIENT_SECRET",
        "LENS_GITHUB_PRIVATE_KEY",
    ];
    if !names
        .iter()
        .any(|name| read(name).is_some_and(|value| !value.trim().is_empty()))
    {
        return Ok(None);
    }
    lens_server::github::GitHubApp::new(
        required(read, names[0])?,
        required(read, names[1])?,
        required(read, names[2])?,
        &required(read, names[3])?,
        public_url
            .parse()
            .map_err(|_| Error::Configuration("LENS_PUBLIC_URL"))?,
    )
    .map(Some)
    .map_err(|_| Error::Configuration("GitHub App settings or private key are invalid"))
}

fn gateway_inference(
    read: &impl Fn(&str) -> Option<String>,
) -> Result<Option<lens_inference::GatewayBase>, Error> {
    let Some(base) = read("LENS_GATEWAY_URL").filter(|base| !base.is_empty()) else {
        return Ok(None);
    };
    let base = base
        .parse()
        .map_err(|_| Error::Configuration("LENS_GATEWAY_URL"))?;
    lens_inference::GatewayBase::new(base)
        .map(Some)
        .map_err(|_| Error::Configuration("LENS_GATEWAY_URL"))
}

fn gateway_models(
    read: &impl Fn(&str) -> Option<String>,
) -> Result<Option<crate::gateway::GatewayConfig>, Error> {
    match (
        read("LENS_GATEWAY_API_BASE").filter(|value| !value.trim().is_empty()),
        read("LENS_GATEWAY_API_KEY").filter(|value| !value.trim().is_empty()),
    ) {
        (None, None) => Ok(None),
        (Some(base), Some(key)) => crate::gateway::GatewayConfig::new(&base, key).map(Some),
        _ => Err(Error::Configuration(
            "Set both LENS_GATEWAY_API_BASE and LENS_GATEWAY_API_KEY",
        )),
    }
}

fn dataset_config(read: impl Fn(&str) -> Option<String>) -> lens_server::datasets::DatasetConfig {
    lens_server::datasets::DatasetConfig {
        limits: lens_datasets::Limits {
            max_cases: integer_or_default(
                read("LENS_DATASET_MAX_CASES").as_deref(),
                lens_datasets::Limits::default().max_cases,
            ),
            max_case_chars: integer_or_default(
                read("LENS_DATASET_MAX_CASE_CHARS").as_deref(),
                lens_datasets::Limits::default().max_case_chars,
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

fn clickhouse_url(read: &impl Fn(&str) -> Option<String>) -> Result<String, Error> {
    if let Ok(url) = required(read, "CLICKHOUSE_URL") {
        return Ok(url);
    }
    if read("CLICKHOUSE_HOST").is_none() {
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
    use super::{Config, dataset_config, integer_or_default};
    use crate::Error;
    use rstest::{fixture, rstest};
    use std::collections::BTreeMap;

    #[rstest]
    #[case::absent(None, None, true, false)]
    #[case::key_only(None, Some("fixture-key"), false, false)]
    #[case::base_only(Some("https://gateway.test"), None, false, false)]
    #[case::connected(Some("https://gateway.test/v1"), Some("fixture-key"), true, true)]
    fn gateway_discovery_requires_both_connection_fields(
        #[case] base: Option<&str>,
        #[case] key: Option<&str>,
        #[case] valid: bool,
        #[case] configured: bool,
    ) {
        let result = super::gateway_models(&|name| match name {
            "LENS_GATEWAY_API_BASE" => base.map(str::to_owned),
            "LENS_GATEWAY_API_KEY" => key.map(str::to_owned),
            _ => None,
        });
        assert_eq!(result.is_ok(), valid);
        if valid {
            assert_eq!(result.unwrap().is_some(), configured);
        }
    }

    #[rstest]
    #[case::absent(None, true, false)]
    #[case::invalid(Some("ftp://gateway.test"), false, false)]
    #[case::valid(Some("https://gateway.test/proxy"), true, true)]
    fn inference_requires_explicit_gateway(
        #[case] url: Option<&str>,
        #[case] valid: bool,
        #[case] configured: bool,
    ) {
        let result = super::gateway_inference(&|name| match name {
            "LENS_GATEWAY_URL" => url.map(str::to_owned),
            _ => None,
        });
        assert_eq!(result.is_ok(), valid);
        if valid {
            assert_eq!(result.unwrap().is_some(), configured);
        }
    }

    #[rstest]
    #[case::localhost(
        "http://localhost:4318",
        Some("true"),
        Some("http://localhost:4318/ui/#token=setup-token+%26%23")
    )]
    #[case::trailing_slash(
        "http://localhost:4318/",
        Some("true"),
        Some("http://localhost:4318/ui/#token=setup-token+%26%23")
    )]
    #[case::loopback_ipv4(
        "http://127.0.0.1:4320",
        Some("true"),
        Some("http://127.0.0.1:4320/ui/#token=setup-token+%26%23")
    )]
    #[case::loopback_ipv6(
        "http://[::1]:4318",
        Some("true"),
        Some("http://[::1]:4318/ui/#token=setup-token+%26%23")
    )]
    #[case::not_requested("http://localhost:4318", None, None)]
    #[case::switched_off("http://localhost:4318", Some("false"), None)]
    #[case::public_host("https://lens.example.com", Some("true"), None)]
    #[case::private_address("http://10.0.0.5:4318", Some("true"), None)]
    #[case::localhost_lookalike("http://localhost.example.com", Some("true"), None)]
    fn sign_in_link_is_only_printed_for_a_requested_local_deployment(
        #[case] public_url: &str,
        #[case] requested: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        let link = super::sign_in_link(
            |name| (name == "LENS_SIGN_IN_LINK").then(|| requested.map(str::to_owned))?,
            &public_url.parse().unwrap(),
            "setup-token &#",
        );
        assert_eq!(link.as_ref().map(url::Url::as_str), expected);
    }

    #[fixture]
    fn standalone() -> BTreeMap<&'static str, &'static str> {
        BTreeMap::from([(
            "LENS_ADMIN_TOKEN",
            "standalone-setup-token-at-least-32-characters",
        )])
    }

    #[rstest]
    fn standalone_requires_no_gateway_settings(standalone: BTreeMap<&str, &str>) {
        let config =
            Config::read(|key| standalone.get(key).map(|value| value.to_string())).unwrap();
        assert_eq!(config.storage.storage().database(), "lens");
        assert_eq!(config.address.to_string(), "0.0.0.0:4318");
        assert_eq!(config.query_secret, standalone["LENS_ADMIN_TOKEN"]);
        assert_eq!(config.ingestion_url, "http://localhost:4318");
        assert_eq!(
            config.signal_alerts_ui_url.as_str(),
            "http://localhost:4318/ui/"
        );
        assert_eq!(config.release, env!("CARGO_PKG_VERSION"));
        assert!(config.github.is_none());
    }

    #[rstest]
    #[case::embedded("https://gateway.example.test/prefix/ui/lens/", true)]
    #[case::relative("/ui/lens/", false)]
    #[case::credentials("https://user:password@gateway.example.test/ui/lens/", false)]
    #[case::query("https://gateway.example.test/ui/lens/?trace=obsolete", false)]
    fn signal_alert_destination_uses_a_public_ui_url(
        standalone: BTreeMap<&str, &str>,
        #[case] setting: &str,
        #[case] valid: bool,
    ) {
        let result = Config::read(|key| {
            if key == "LENS_SIGNAL_ALERTS_UI_URL" {
                Some(setting.to_string())
            } else {
                standalone.get(key).map(|value| value.to_string())
            }
        });
        if valid {
            let config = result.unwrap();
            assert_eq!(config.signal_alerts_ui_url.as_str(), setting);
            assert_eq!(config.public_url.as_str(), "http://localhost:4318/");
        } else {
            assert!(matches!(
                result,
                Err(Error::Configuration("LENS_SIGNAL_ALERTS_UI_URL"))
            ));
        }
    }

    #[rstest]
    #[case::partial(false, false)]
    #[case::invalid_key(true, false)]
    #[case::configured(true, true)]
    fn github_requires_complete_valid_credentials(#[case] complete: bool, #[case] valid_key: bool) {
        let settings = BTreeMap::from([
            ("LENS_GITHUB_APP_SLUG", "lens-test"),
            ("LENS_GITHUB_CLIENT_ID", "client-id"),
            ("LENS_GITHUB_CLIENT_SECRET", "client-secret"),
            (
                "LENS_GITHUB_PRIVATE_KEY",
                if valid_key {
                    include_str!("../../server/tests/fixtures/github-test-key.pem")
                } else {
                    "invalid-pem"
                },
            ),
        ]);
        let result = super::github(
            &|name| {
                if !complete && name != "LENS_GITHUB_APP_SLUG" {
                    None
                } else {
                    settings.get(name).map(|value| value.to_string())
                }
            },
            "https://lens.test",
        );
        assert_eq!(result.is_ok(), complete && valid_key);
        if complete && valid_key {
            assert!(result.unwrap().is_some());
        }
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
    #[case::implicit(None)]
    #[case::explicit(Some("standalone"))]
    fn legacy_gateway_variables_cannot_change_storage_or_authentication(
        standalone: BTreeMap<&str, &str>,
        #[case] mode: Option<&str>,
    ) {
        let config = Config::read(|key| match key {
            "LENS_MODE" => mode.map(str::to_owned),
            "LITELLM_URL" => Some("invalid-gateway-url".into()),
            "LITELLM_RELEASE_TAG" => Some("gateway-release".into()),
            _ => standalone.get(key).map(|value| value.to_string()),
        })
        .unwrap();
        assert_eq!(config.storage.storage().database(), "lens");
        assert_eq!(config.query_secret, standalone["LENS_ADMIN_TOKEN"]);
        assert_eq!(config.public_url.as_str(), "http://localhost:4318/");
        assert_eq!(config.release, env!("CARGO_PKG_VERSION"));
    }

    #[rstest]
    #[case::retired_gateway("gateway")]
    #[case::unknown("other")]
    #[case::empty("")]
    fn unsupported_mode_is_rejected(#[case] mode: &str) {
        let result = Config::read(|key| (key == "LENS_MODE").then(|| mode.into()));
        assert!(matches!(
            result,
            Err(Error::Configuration(
                "Lens runs as a standalone service; remove LENS_MODE"
            ))
        ));
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
    #[case::ingress_prefix(
        Some("https://gateway.example.test/lens-ingest/"),
        "https://gateway.example.test/lens-ingest"
    )]
    fn standalone_advertises_the_configured_ingestion_url(
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
        assert!(config.authentication.secure_cookie());
    }

    #[rstest]
    #[case::scheme("file:///tmp/traces")]
    #[case::credentials("https://user:password@lens.example.test")]
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
    #[case::absent(None, true)]
    #[case::configured(Some("optional-gateway-service-token-32-chars"), true)]
    #[case::empty(Some(""), false)]
    #[case::short(Some("short-private-value"), false)]
    fn standalone_gateway_connection_is_optional_and_uses_a_bounded_secret(
        standalone: BTreeMap<&str, &str>,
        #[case] token: Option<&str>,
        #[case] valid: bool,
    ) {
        let config = Config::read(|name| match name {
            "LITELLM_LENS_SERVICE_TOKEN" => token.map(str::to_owned),
            _ => standalone.get(name).map(|value| value.to_string()),
        });
        if valid {
            let config = config.unwrap();
            assert_eq!(config.gateway_service_token.as_deref(), token);
        } else {
            let error = config.err().unwrap();
            assert!(matches!(error, Error::Configuration(_)));
            assert!(!error.to_string().contains("short-private-value"));
        }
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
        assert_eq!(
            config.limits.max_cases,
            lens_datasets::Limits::default().max_cases
        );
        assert_eq!(
            config.limits.max_case_chars,
            lens_datasets::Limits::default().max_case_chars
        );
        assert_eq!(config.trace_retry_after_seconds, 2);
    }
}
