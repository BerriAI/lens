use std::{collections::BTreeMap, path::Path, time::Instant};

use lens_parity::{
    BenchmarkError,
    benchmark::{Distribution, Report, Stage, compare, fixture, trace_id},
};
use reqwest::{Client, Method};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    base_url: String,
    #[serde(default)]
    ingestion_url: Option<String>,
    admin_token: String,
    tracing_key: String,
    source_sha: String,
    artifact: String,
    resources: String,
    timestamp_ms: i64,
}

struct Service {
    config: Config,
    base: Url,
    ingestion: Url,
    client: Client,
}

impl Service {
    async fn request(
        &self,
        method: Method,
        path: &str,
        ingestion: bool,
        body: Option<Value>,
    ) -> Result<(Value, f64), BenchmarkError> {
        let (base, token) = if ingestion {
            (&self.ingestion, &self.config.tracing_key)
        } else {
            (&self.base, &self.config.admin_token)
        };
        let request = self
            .client
            .request(method, base.join(path)?)
            .bearer_auth(token);
        let request = if let Some(body) = body {
            request.json(&body)
        } else {
            request
        };
        let start = Instant::now();
        let response = request.send().await?.error_for_status()?;
        let body = response.json().await?;
        Ok((body, start.elapsed().as_secs_f64() * 1000.0))
    }

    async fn list(&self, expected: usize) -> Result<f64, BenchmarkError> {
        let mut cursor: Option<String> = None;
        let mut rows = BTreeMap::new();
        let mut elapsed = 0.0;
        loop {
            let mut url = self.base.join("v1/traces")?;
            url.query_pairs_mut()
                .append_pair("start_ms", &(self.config.timestamp_ms - 1).to_string())
                .append_pair("end_ms", &(self.config.timestamp_ms + 100_001).to_string());
            if let Some(cursor) = &cursor {
                url.query_pairs_mut().append_pair("cursor", cursor);
            }
            let (body, duration) = self.request(Method::GET, url.as_str(), false, None).await?;
            elapsed += duration;
            let data = body["data"]
                .as_array()
                .ok_or(BenchmarkError::Response("trace list"))?;
            for row in data {
                let id = row["trace_id"]
                    .as_str()
                    .ok_or(BenchmarkError::Response("trace identifier"))?;
                let count = row["span_count"]
                    .as_u64()
                    .ok_or(BenchmarkError::Response("span count"))?;
                if rows.insert(id.to_owned(), count).is_some() {
                    return Err(BenchmarkError::Response("duplicate trace"));
                }
            }
            cursor = body["next_cursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
            if rows.len() > expected {
                return Err(BenchmarkError::Response("unexpected trace"));
            }
        }
        let wanted: BTreeMap<_, _> = (0..expected).map(|index| (trace_id(index), 10)).collect();
        if rows != wanted {
            return Err(BenchmarkError::Response("trace or span loss"));
        }
        Ok(elapsed)
    }

    async fn receipt(&self, index: usize) -> Result<f64, BenchmarkError> {
        let spans: Vec<_> = (1..=10).map(|id| format!("{id:016x}")).collect();
        let (body, elapsed) = self
            .request(
                Method::POST,
                "v1/traces/receipt",
                true,
                Some(json!({"trace_id":trace_id(index),"span_ids":spans})),
            )
            .await?;
        if body["received"] != true {
            return Err(BenchmarkError::Response("delivery receipt"));
        }
        Ok(elapsed)
    }

    async fn detail(&self, index: usize) -> Result<f64, BenchmarkError> {
        let (body, elapsed) = self
            .request(
                Method::GET,
                &format!("v1/traces/{}", trace_id(index)),
                false,
                None,
            )
            .await?;
        let spans = body["spans"]
            .as_array()
            .ok_or(BenchmarkError::Response("trace details"))?;
        let ids: std::collections::BTreeSet<_> = spans
            .iter()
            .filter_map(|span| span["span_id"].as_str())
            .collect();
        let expected: std::collections::BTreeSet<_> =
            (1..=10).map(|id| format!("{id:016x}")).collect();
        if ids.iter().copied().ne(expected.iter().map(String::as_str))
            || !body["next_cursor"].is_null()
        {
            return Err(BenchmarkError::Response("complete trace graph"));
        }
        Ok(elapsed)
    }

    async fn content(&self, index: usize) -> Result<f64, BenchmarkError> {
        let (body, elapsed) = self
            .request(
                Method::GET,
                &format!("v1/traces/{}/spans/0000000000000001", trace_id(index)),
                false,
                None,
            )
            .await?;
        if body["input"] != format!("benchmark request {index}")
            || body["output"] != "benchmark response"
        {
            return Err(BenchmarkError::Response("exported span content"));
        }
        Ok(elapsed)
    }

    async fn run(&self) -> Result<Report, BenchmarkError> {
        if self.config.source_sha.len() != 40
            || !self
                .config
                .source_sha
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.config.artifact.is_empty()
            || self.config.resources.is_empty()
        {
            return Err(BenchmarkError::Response(
                "source, artifact and resource identity",
            ));
        }
        self.list(0).await?;
        let mut stages = Vec::new();
        let mut previous = 0;
        for total in [100, 1000] {
            let mut ingestion = Vec::new();
            for start in (previous..total).step_by(10) {
                let (_, elapsed) = self
                    .request(
                        Method::POST,
                        "v1/traces",
                        true,
                        Some(fixture(start..start + 10, self.config.timestamp_ms)),
                    )
                    .await?;
                ingestion.push(elapsed);
            }
            self.receipt(total - 1).await?;
            self.list(total).await?;
            self.detail(total - 1).await?;
            self.content(total - 1).await?;
            let mut list = Vec::new();
            let mut detail = Vec::new();
            let mut content = Vec::new();
            let mut receipt = Vec::new();
            for _ in 0..11 {
                receipt.push(self.receipt(total - 1).await?);
                list.push(self.list(total).await?);
                detail.push(self.detail(total - 1).await?);
                content.push(self.content(total - 1).await?);
            }
            stages.push(Stage {
                traces: total,
                spans: total * 10,
                measurements: BTreeMap::from([
                    ("ingest_10_traces".into(), Distribution::new(ingestion)?),
                    ("receipt".into(), Distribution::new(receipt)?),
                    ("list_all_traces".into(), Distribution::new(list)?),
                    ("trace_details".into(), Distribution::new(detail)?),
                    ("span_content".into(), Distribution::new(content)?),
                ]),
            });
            previous = total;
        }
        Ok(Report {
            schema_version: 1,
            source_sha: self.config.source_sha.clone(),
            artifact: self.config.artifact.clone(),
            resources: self.config.resources.clone(),
            timestamp_ms: self.config.timestamp_ms,
            stages,
        })
    }
}

fn read<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, BenchmarkError> {
    Ok(serde_json::from_slice(&std::fs::read(Path::new(path))?)?)
}

fn service_url(value: &str) -> Result<Url, BenchmarkError> {
    let base = Url::parse(value)?;
    if !matches!(base.scheme(), "http" | "https")
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err(BenchmarkError::Response(
            "HTTP service URL without credentials, query or fragment",
        ));
    }
    Ok(base)
}

#[tokio::main]
async fn main() -> Result<(), BenchmarkError> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, config] if command == "run" => {
            let config: Config = read(config)?;
            let base = service_url(&config.base_url)?;
            let ingestion =
                service_url(config.ingestion_url.as_deref().unwrap_or(&config.base_url))?;
            let service = Service {
                config,
                base,
                ingestion,
                client: Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()?,
            };
            println!("{}", serde_json::to_string_pretty(&service.run().await?)?);
        }
        [command, baseline, candidate] if command == "compare" => {
            let result = compare(&read(baseline)?, &read(candidate)?)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            if !result.passed {
                return Err(BenchmarkError::Response("paired latency budget"));
            }
        }
        _ => {
            return Err(BenchmarkError::Response(
                "usage: service-benchmark run PRIVATE_CONFIG.json | compare BASELINE.json CANDIDATE.json",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Config, Service, service_url};
    use reqwest::{Client, Method};
    use rstest::{fixture, rstest};
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };

    #[fixture]
    fn config() -> Config {
        Config {
            base_url: "http://localhost/".into(),
            ingestion_url: None,
            admin_token: "admin-credential".into(),
            tracing_key: "tracing-credential".into(),
            source_sha: "a".repeat(40),
            artifact: "test".into(),
            resources: "test".into(),
            timestamp_ms: 0,
        }
    }

    #[rstest]
    #[case::read(false, "/read/probe", "Bearer admin-credential")]
    #[case::ingestion(true, "/ingest/probe", "Bearer tracing-credential")]
    #[tokio::test]
    async fn requests_use_the_selected_service_and_credential(
        config: Config,
        #[case] ingestion: bool,
        #[case] expected_path: &str,
        #[case] credential: &str,
    ) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(expected_path))
            .and(header("authorization", credential))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"received":true})))
            .expect(1)
            .mount(&server)
            .await;
        let service = Service {
            config,
            base: service_url(&format!("{}/read/", server.uri())).unwrap(),
            ingestion: service_url(&format!("{}/ingest/", server.uri())).unwrap(),
            client: Client::new(),
        };
        let (body, elapsed) = service
            .request(
                Method::POST,
                "probe",
                ingestion,
                Some(json!({"probe":true})),
            )
            .await
            .unwrap();
        assert_eq!(body, json!({"received":true}));
        assert!(elapsed >= 0.0);
    }

    #[rstest]
    #[case::http("http://localhost/prefix/", true)]
    #[case::https("https://example.test/prefix/", true)]
    #[case::scheme("file:///tmp/", false)]
    #[case::username("http://user@example.test/", false)]
    #[case::password("http://:password@example.test/", false)]
    #[case::query("http://example.test/?key=value", false)]
    #[case::fragment("http://example.test/#fragment", false)]
    fn service_urls_preserve_prefixes_and_reject_embedded_credentials(
        #[case] input: &str,
        #[case] valid: bool,
    ) {
        let result = service_url(input);
        assert_eq!(result.is_ok(), valid);
        if let Ok(url) = result {
            assert_eq!(url.as_str(), input);
        }
    }
}
