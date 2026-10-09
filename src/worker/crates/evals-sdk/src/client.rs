use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::model::{CaseResult, CreateEvalRun, EvalCases, EvalRun, ResolvedDataset};
use crate::{Error, Result};

#[derive(Deserialize)]
struct LegacyDataset {
    id: String,
    name: String,
    revision: u64,
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    endpoint: String,
    key: String,
    attempts: usize,
    retry_delay: Duration,
    contract: &'static str,
}

pub fn endpoint(value: &str) -> Result<String> {
    let parsed = Url::parse(value).map_err(|_| {
        Error::Configuration("Set LENS_BASE_URL or [tool.lens].base_url to the Lens server URL")
    })?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(Error::Configuration(
            "Lens URL requires HTTP(S), a host, and no credentials, query or fragment",
        ));
    }
    if parsed.scheme() == "http"
        && !matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
    {
        return Err(Error::Configuration("Remote Lens servers require HTTPS"));
    }
    Ok(value.trim_end_matches('/').to_owned())
}

impl Client {
    pub fn new(base: &str, key: &str) -> Result<Self> {
        if key.trim().is_empty() {
            return Err(Error::Configuration("Set LENS_API_KEY"));
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(40))
            .build()
            .map_err(Error::Transport)?;
        Ok(Self {
            http,
            endpoint: endpoint(base)?,
            key: key.to_owned(),
            attempts: 3,
            retry_delay: Duration::from_millis(250),
            contract: "1",
        })
    }

    pub fn named(base: &str, key: &str) -> Result<Self> {
        Ok(Self {
            contract: "2",
            ..Self::new(base, key)?
        })
    }

    pub async fn definition(&self, name: &str) -> Result<lens_contract::eval::EvalDefinition> {
        let definition: lens_contract::eval::EvalDefinition = Self::decode(
            self.request(
                Method::GET,
                &format!("/lens/evals/{}", segment(name)),
                None,
                None,
                true,
            )
            .await?,
        )
        .await?;
        if definition.name != name {
            return Err(Error::Infrastructure(
                "Lens returned a different saved eval",
            ));
        }
        Ok(definition)
    }

    pub async fn dataset(&self, id: &str, revision: Option<u64>) -> Result<ResolvedDataset> {
        if let Some(revision) = revision {
            if revision == 0 {
                return Err(Error::Configuration("Dataset revision must be positive"));
            }
            return Ok(ResolvedDataset {
                id: id.to_owned(),
                name: String::new(),
                revision,
            });
        }
        let dataset: LegacyDataset = Self::decode(
            self.request(
                Method::GET,
                &format!("/lens/datasets/{}", segment(id)),
                None,
                None,
                true,
            )
            .await?,
        )
        .await?;
        if dataset.id != id || dataset.revision == 0 {
            return Err(Error::Infrastructure(
                "Lens returned a different dataset or invalid revision",
            ));
        }
        Ok(ResolvedDataset {
            id: dataset.id,
            name: dataset.name,
            revision: dataset.revision,
        })
    }

    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        idempotency: Option<&str>,
        retry: bool,
    ) -> Result<reqwest::Response> {
        let limit = if retry { self.attempts } else { 1 };
        for attempt in 0..limit {
            let request = self
                .http
                .request(method.clone(), format!("{}{path}", self.endpoint))
                .bearer_auth(&self.key)
                .header("X-Lens-Contract", self.contract)
                .header("Content-Type", "application/json");
            let request = if let Some(value) = body {
                request.json(value)
            } else {
                request
            };
            let request = if let Some(value) = idempotency {
                request.header("Idempotency-Key", value)
            } else {
                request
            };
            let can_retry = attempt + 1 < limit;
            let response = match request.send().await {
                Ok(response) => response,
                Err(_) if can_retry => {
                    tokio::time::sleep(self.retry_delay * 2_u32.pow(attempt as u32)).await;
                    continue;
                }
                Err(error) => return Err(Error::Transport(error)),
            };
            let status = response.status();
            if matches!(status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504) && can_retry {
                tokio::time::sleep(self.retry_delay * 2_u32.pow(attempt as u32)).await;
                continue;
            }
            if status.is_redirection() {
                return Err(Error::Infrastructure(
                    "Lens returned an unexpected redirect",
                ));
            }
            if status.is_client_error() || status.is_server_error() {
                let value = response.json::<Value>().await.unwrap_or(Value::Null);
                let code = value
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("request_failed")
                    .to_owned();
                return Err(Error::Api {
                    status: status.as_u16(),
                    code,
                });
            }
            return Ok(response);
        }
        Err(Error::Infrastructure(
            "Lens request exhausted its retry budget",
        ))
    }

    pub async fn decode<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
        let bytes = response.bytes().await.map_err(Error::Transport)?;
        serde_json::from_slice(&bytes).map_err(Error::Response)
    }

    pub async fn resolve(&self, data: &str) -> Result<ResolvedDataset> {
        let (name, revision) = match data.rsplit_once('@') {
            Some((name, text)) => (
                name,
                Some(
                    text.parse::<u64>()
                        .ok()
                        .filter(|revision| *revision > 0)
                        .ok_or(Error::Configuration(
                            "Use a dataset name or name@positive-revision",
                        ))?,
                ),
            ),
            None => (data, None),
        };
        if name.is_empty() {
            return Err(Error::Configuration(
                "Use a dataset name or name@positive-revision",
            ));
        }
        let mut url = Url::parse("http://localhost/lens/datasets/resolve").expect("fixed URL");
        url.query_pairs_mut().append_pair("name", name);
        if let Some(revision) = revision {
            url.query_pairs_mut()
                .append_pair("revision", &revision.to_string());
        }
        let path = &url[url::Position::BeforePath..];
        let resolved: ResolvedDataset =
            match self.request(Method::GET, path, None, None, true).await {
                Ok(response) => Self::decode(response).await?,
                Err(Error::Api { status: 404, code })
                    if matches!(code.as_str(), "request_failed" | "dataset_not_found") =>
                {
                    let values: Vec<LegacyDataset> = Self::decode(
                        self.request(Method::GET, "/lens/datasets", None, None, true)
                            .await?,
                    )
                    .await?;
                    let matches = values
                        .into_iter()
                        .filter(|value| value.name == name)
                        .collect::<Vec<_>>();
                    if matches.len() != 1 {
                        return Err(Error::Invalid {
                            message: "Dataset name must identify exactly one accessible dataset",
                            value: name.to_owned(),
                        });
                    }
                    ResolvedDataset {
                        id: matches[0].id.clone(),
                        name: matches[0].name.clone(),
                        revision: revision.unwrap_or(matches[0].revision),
                    }
                }
                Err(error) => return Err(error),
            };
        if resolved.name != name
            || revision.is_some_and(|value| value != resolved.revision)
            || resolved.revision == 0
        {
            return Err(Error::Infrastructure(
                "Lens resolved a different dataset or revision than requested",
            ));
        }
        Ok(resolved)
    }

    pub async fn cases(&self, dataset: &ResolvedDataset) -> Result<EvalCases> {
        let data: EvalCases = Self::decode(
            self.request(
                Method::GET,
                &format!(
                    "/lens/datasets/{}/revisions/{}/cases",
                    segment(&dataset.id),
                    dataset.revision
                ),
                None,
                None,
                true,
            )
            .await?,
        )
        .await?;
        if data.dataset_id != dataset.id || data.revision != dataset.revision {
            return Err(Error::Infrastructure(
                "Lens returned cases from a different dataset revision",
            ));
        }
        Ok(data)
    }

    pub async fn create(&self, body: &CreateEvalRun, key: &str) -> Result<EvalRun> {
        Self::decode(
            self.request(
                Method::POST,
                "/lens/evals/runs",
                Some(&value(body)?),
                Some(key),
                true,
            )
            .await?,
        )
        .await
    }

    pub async fn result(
        &self,
        run_id: &str,
        case_id: &str,
        trial: usize,
        result: &CaseResult,
    ) -> Result<()> {
        result.validate()?;
        self.request(
            Method::PUT,
            &format!(
                "/lens/evals/runs/{}/results/{}/{trial}",
                segment(run_id),
                segment(case_id)
            ),
            Some(&value(result)?),
            None,
            true,
        )
        .await?;
        Ok(())
    }

    pub async fn get(&self, run_id: &str, wait: bool) -> Result<EvalRun> {
        let run: EvalRun = Self::decode(
            self.request(
                Method::GET,
                &format!(
                    "/lens/evals/runs/{}{}",
                    segment(run_id),
                    if wait { "?wait=30" } else { "" }
                ),
                None,
                None,
                true,
            )
            .await?,
        )
        .await?;
        if run.id != run_id {
            return Err(Error::Infrastructure("Lens returned a different eval run"));
        }
        Ok(run)
    }

    pub async fn finish(&self, run_id: &str) -> Result<EvalRun> {
        match self
            .request(
                Method::POST,
                &format!("/lens/evals/runs/{}/finish", segment(run_id)),
                None,
                None,
                true,
            )
            .await
        {
            Ok(response) => {
                let run: EvalRun = Self::decode(response).await?;
                if run.id != run_id {
                    return Err(Error::Infrastructure("Lens finished a different eval run"));
                }
                Ok(run)
            }
            Err(Error::Api { status, code })
                if status == StatusCode::CONFLICT.as_u16() && code == "run_closed" =>
            {
                self.get(run_id, false).await
            }
            Err(error) => Err(error),
        }
    }
}

pub fn value<T: Serialize>(value: &T) -> Result<Value> {
    serde_json::to_value(value).map_err(Error::Response)
}

pub fn segment(value: &str) -> String {
    let mut url = Url::parse("http://localhost/").expect("fixed URL");
    url.path_segments_mut()
        .expect("HTTP URL has segments")
        .push(value);
    url.path().trim_start_matches('/').to_owned()
}
