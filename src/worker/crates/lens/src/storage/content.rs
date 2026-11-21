use std::{collections::BTreeMap, sync::Arc, time::Duration};

use litellm_storage_clickhouse::fetch;
use litellm_traces::{ReadQuery, SpanDetail, to_ui_content};
use litellm_traces_clickhouse::{
    CONTENT_REFERENCE_ATTRIBUTE, Parameter,
    content_reference::{self, ContentReference, Owner},
    query::lens::{
        ContentSource, LensAccessParams, LensContentParams, LensEvidenceParams,
        SpendContentReference, SpendContentReferenceParams,
    },
};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Semaphore;

use super::Storage;
use crate::Error;

const MAX_CONCURRENT_READS: usize = 4;
const READ_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub struct ContentGateway {
    client: reqwest::Client,
    endpoint: Option<url::Url>,
    token: Arc<str>,
    key: [u8; 32],
    slots: Arc<Semaphore>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Hydrated {
    Content { messages: String, response: String },
    Unavailable(String),
}

#[derive(Deserialize)]
struct StoredContent {
    messages: String,
    response: String,
}

#[derive(Deserialize)]
struct SpanReference {
    team_id: String,
    api_key: String,
    request_id: String,
    reference: String,
}

impl ContentGateway {
    pub fn new(base: Option<&url::Url>, service_token: &str) -> Result<Self, Error> {
        let endpoint = base
            .map(|base| {
                format!("{}/lens/content", base.as_str().trim_end_matches('/'))
                    .parse()
                    .map_err(|_| Error::Configuration("LENS_GATEWAY_API_BASE"))
            })
            .transpose()?;
        let config = litellm_http::Resolution::from(&litellm_http::HttpSettings::default()).config;
        let client = reqwest::ClientBuilder::try_from(&config)?
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(READ_TIMEOUT)
            .build()?;
        Ok(Self {
            client,
            endpoint,
            token: service_token.into(),
            key: content_reference::signing_key(service_token),
            slots: Arc::new(Semaphore::new(MAX_CONCURRENT_READS)),
        })
    }

    pub fn from_config(config: &crate::config::Config) -> Result<Option<Self>, Error> {
        config
            .gateway_service_token
            .as_deref()
            .map(|token| {
                Self::new(
                    config.gateway_models.as_ref().map(|gateway| &gateway.base),
                    token,
                )
            })
            .transpose()
    }

    pub async fn hydrate(&self, owner: &Owner<'_>, raw: &str) -> Option<Hydrated> {
        match content_reference::verify(&self.key, owner, raw)? {
            ContentReference::Unavailable(reason) => Some(Hydrated::Unavailable(reason)),
            ContentReference::Stored { bytes, sha256, .. } => Some(
                self.download(owner, raw, bytes, &sha256)
                    .await
                    .unwrap_or_else(|reason| Hydrated::Unavailable(reason.into())),
            ),
        }
    }

    async fn download(
        &self,
        owner: &Owner<'_>,
        raw: &str,
        bytes: u64,
        sha256: &str,
    ) -> Result<Hydrated, &'static str> {
        let endpoint = self.endpoint.clone().ok_or("retrieval_not_configured")?;
        let _permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "retrieval_overloaded")?;
        let reference: serde_json::Value =
            serde_json::from_str(raw).map_err(|_| "retrieval_failed")?;
        let mut response = self
            .client
            .post(endpoint)
            .bearer_auth(&*self.token)
            .json(&serde_json::json!({
                "team_id": owner.team_id,
                "api_key": owner.api_key,
                "request_id": owner.request_id,
                "reference": reference,
            }))
            .send()
            .await
            .map_err(|_| "retrieval_failed")?;
        if !response.status().is_success() {
            return Err(match response.status().as_u16() {
                409 => "integrity_failed",
                _ => "retrieval_failed",
            });
        }
        let limit = usize::try_from(bytes).map_err(|_| "too_large")?;
        let mut body = Vec::with_capacity(limit.min(64 * 1024));
        while let Some(chunk) = response.chunk().await.map_err(|_| "retrieval_failed")? {
            if body.len() + chunk.len() > limit {
                return Err("integrity_failed");
            }
            body.extend_from_slice(&chunk);
        }
        if !content_reference::intact(&body, bytes, sha256) {
            return Err("integrity_failed");
        }
        let content: StoredContent =
            serde_json::from_slice(&body).map_err(|_| "integrity_failed")?;
        Ok(Hydrated::Content {
            messages: content.messages,
            response: content.response,
        })
    }
}

async fn hydrate(
    content: Option<&ContentGateway>,
    owner: &Owner<'_>,
    raw: &str,
) -> Option<Hydrated> {
    content?.hydrate(owner, raw).await
}

fn parts(hydrated: Hydrated) -> (String, String) {
    match hydrated {
        Hydrated::Content { messages, response } => (messages, response),
        Hydrated::Unavailable(reason) => {
            (format!("[content unavailable: {reason}]"), String::new())
        }
    }
}

fn reference_params(
    access: &LensAccessParams,
    id: &str,
    record_team: &str,
    start_time: &str,
) -> SpendContentReferenceParams {
    SpendContentReferenceParams {
        access: LensAccessParams {
            all_teams: access.all_teams,
            team: access.team.clone(),
            key_hash: access.key_hash.clone(),
        },
        id: id.into(),
        record_team: record_team.into(),
        start_time: start_time.into(),
    }
}

fn proves(spend: &SpendContent, quote: &str) -> bool {
    match &spend.hydrated {
        Hydrated::Content { messages, response } => {
            content_reference::page(messages, response, &spend.error, 0)
                .0
                .contains(quote)
        }
        Hydrated::Unavailable(_) => false,
    }
}

pub(crate) struct HydratedPart {
    pub span_id: String,
    pub content: String,
    pub truncated: u8,
}

struct SpendContent {
    request_id: String,
    error: String,
    hydrated: Hydrated,
}

impl Storage {
    pub fn with_content(mut self, content: Option<ContentGateway>) -> Self {
        self.content = content;
        self
    }

    async fn spend_content(
        &self,
        params: SpendContentReferenceParams,
    ) -> Result<Option<SpendContent>, Error> {
        let rows =
            fetch::<SpendContentReference>(&self.client, self.config.storage().reader(), &params)
                .await?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        let owner = Owner {
            team_id: &row.team_id,
            api_key: &row.api_key,
            request_id: &row.request_id,
        };
        let Some(hydrated) = hydrate(self.content.as_ref(), &owner, &row.reference).await else {
            return Ok(None);
        };
        Ok(Some(SpendContent {
            request_id: row.request_id,
            error: row.error_str,
            hydrated,
        }))
    }

    pub(crate) async fn hydrated_part(
        &self,
        params: &LensContentParams,
    ) -> Result<Option<HydratedPart>, Error> {
        if !matches!(params.source, ContentSource::Requests) {
            return Ok(None);
        }
        let Some(spend) = self
            .spend_content(reference_params(
                &params.access,
                &params.id,
                &params.record_team,
                &params.start_time,
            ))
            .await?
        else {
            return Ok(None);
        };
        let (messages, response) = parts(spend.hydrated);
        let (content, truncated) =
            content_reference::page(&messages, &response, &spend.error, params.offset);
        Ok(Some(HydratedPart {
            span_id: spend.request_id,
            content,
            truncated,
        }))
    }

    pub(crate) async fn hydrated_evidence(
        &self,
        params: &LensEvidenceParams,
    ) -> Result<bool, Error> {
        if !matches!(params.source, ContentSource::Requests) || params.span != params.id {
            return Ok(false);
        }
        let spend = self
            .spend_content(reference_params(
                &params.access,
                &params.id,
                &params.record_team,
                &params.start_time,
            ))
            .await?;
        Ok(spend.is_some_and(|spend| proves(&spend, &params.quote)))
    }

    pub(crate) async fn hydrate_named(
        &self,
        query: ReadQuery,
        parameters: &BTreeMap<String, Parameter>,
        mut value: Value,
    ) -> Result<Value, Error> {
        let Ok(parameters) = serde_json::to_value(parameters) else {
            return Ok(value);
        };
        match query {
            ReadQuery::Content => {
                let Ok(params) = serde_json::from_value::<LensContentParams>(parameters) else {
                    return Ok(value);
                };
                let Some(part) = self.hydrated_part(&params).await? else {
                    return Ok(value);
                };
                let rows = value.get_mut("data").and_then(Value::as_array_mut);
                for row in rows.into_iter().flatten().filter(|row| {
                    row.get("span_id").and_then(Value::as_str) == Some(part.span_id.as_str())
                }) {
                    row["content"] = Value::from(part.content.clone());
                    row["truncated"] = Value::from(part.truncated);
                }
            }
            ReadQuery::Evidence => {
                let Ok(params) = serde_json::from_value::<LensEvidenceParams>(parameters) else {
                    return Ok(value);
                };
                let unmatched = value
                    .pointer("/data/0/count")
                    .is_some_and(|count| count.as_str() == Some("0") || count.as_u64() == Some(0));
                if unmatched
                    && self.hydrated_evidence(&params).await?
                    && let Some(count) = value.pointer_mut("/data/0/count")
                {
                    *count = if count.is_string() {
                        Value::from("1")
                    } else {
                        Value::from(1)
                    };
                }
            }
            _ => {}
        }
        Ok(value)
    }

    pub(crate) async fn hydrate_span(&self, detail: Option<SpanDetail>) -> Option<SpanDetail> {
        let mut detail = detail?;
        let Some(raw) = detail.attributes.remove(CONTENT_REFERENCE_ATTRIBUTE) else {
            return Some(detail);
        };
        let Ok(reference) = serde_json::from_str::<SpanReference>(&raw) else {
            return Some(detail);
        };
        let owner = Owner {
            team_id: &reference.team_id,
            api_key: &reference.api_key,
            request_id: &reference.request_id,
        };
        match hydrate(self.content.as_ref(), &owner, &reference.reference).await {
            Some(Hydrated::Content { messages, response }) => {
                detail.input_ui = to_ui_content(&messages);
                detail.output_ui = to_ui_content(&response);
                detail.input = messages;
                detail.output = response;
                detail
                    .attributes
                    .insert("litellm.content.state".into(), "stored".into());
            }
            Some(Hydrated::Unavailable(reason)) => {
                detail
                    .attributes
                    .insert("litellm.content.state".into(), "unavailable".into());
                detail
                    .attributes
                    .insert("litellm.content.reason".into(), reason);
            }
            None => {}
        }
        Some(detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, header, method, path},
    };

    const TOKEN: &str = "test-service-token-with-32-characters";
    const OWNER: Owner<'static> = Owner {
        team_id: "team-a",
        api_key: "key-a",
        request_id: "request-a",
    };
    const STORED: &[u8] = "{\"messages\":\"\u{754c} prompt\",\"response\":\"answer\"}".as_bytes();

    fn reference(owner: &Owner<'_>, body: &[u8]) -> String {
        content_reference::stored_reference(
            &content_reference::signing_key(TOKEN),
            owner,
            "logs/lens-content/object.json",
            body.len() as u64,
            &format!("{:x}", Sha256::digest(body)),
        )
    }

    async fn gateway(served: &[u8], expected_calls: u64) -> (MockServer, ContentGateway) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/lens/content"))
            .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
            .and(body_partial_json(serde_json::json!({
                "team_id": "team-a", "api_key": "key-a", "request_id": "request-a",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(served.to_vec()))
            .expect(expected_calls)
            .mount(&server)
            .await;
        let gateway = ContentGateway::new(Some(&server.uri().parse().unwrap()), TOKEN).unwrap();
        (server, gateway)
    }

    #[tokio::test]
    async fn stored_content_is_read_through_the_authenticated_gateway() {
        let (server, gateway) = gateway(STORED, 1).await;
        assert_eq!(
            gateway.hydrate(&OWNER, &reference(&OWNER, STORED)).await,
            Some(Hydrated::Content {
                messages: "\u{754c} prompt".into(),
                response: "answer".into(),
            })
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn tampered_objects_are_reported_without_their_content() {
        let tampered = "{\"messages\":\"\u{754c} prompX\",\"response\":\"answer\"}".as_bytes();
        let (server, gateway) = gateway(tampered, 1).await;
        assert_eq!(
            gateway.hydrate(&OWNER, &reference(&OWNER, STORED)).await,
            Some(Hydrated::Unavailable("integrity_failed".into()))
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn foreign_or_forged_references_never_reach_the_gateway() {
        let (server, gateway) = gateway(STORED, 0).await;
        let other_tenant = Owner {
            team_id: "team-b",
            ..OWNER
        };
        assert_eq!(
            gateway
                .hydrate(&OWNER, &reference(&other_tenant, STORED))
                .await,
            None
        );
        let unsigned = serde_json::json!({
            "v": 1, "state": "stored", "object_key": "logs/other.json",
            "bytes": STORED.len(), "sha256": "ab", "mac": "00".repeat(32),
        })
        .to_string();
        assert_eq!(gateway.hydrate(&OWNER, &unsigned).await, None);
        server.verify().await;
    }

    #[test]
    fn evidence_requires_original_content_not_the_unavailable_placeholder() {
        let unavailable = SpendContent {
            request_id: "request-a".into(),
            error: String::new(),
            hydrated: Hydrated::Unavailable("upload_failed".into()),
        };
        assert!(!proves(
            &unavailable,
            "[content unavailable: upload_failed]"
        ));
        assert!(!proves(&unavailable, "Input: "));
        let stored = SpendContent {
            hydrated: Hydrated::Content {
                messages: "\u{754c} prompt".into(),
                response: "answer".into(),
            },
            ..unavailable
        };
        assert!(proves(&stored, "\u{754c} prompt\nOutput: answer"));
        assert!(!proves(&stored, "[content unavailable"));
    }

    #[tokio::test]
    async fn unconfigured_retrieval_trusts_only_signed_states() {
        let gateway = ContentGateway::new(None, TOKEN).unwrap();
        assert_eq!(
            gateway.hydrate(&OWNER, &reference(&OWNER, STORED)).await,
            Some(Hydrated::Unavailable("retrieval_not_configured".into()))
        );
        let forged = serde_json::json!({
            "v": 1, "state": "unavailable", "reason": "too_large", "mac": "00".repeat(32),
        })
        .to_string();
        assert_eq!(gateway.hydrate(&OWNER, &forged).await, None);
        assert_eq!(
            hydrate(None, &OWNER, &reference(&OWNER, STORED)).await,
            None
        );
    }
}
