use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
    time::Duration,
};

use litellm_traces::{SpanDetail, hydrated_input_output, to_ui_content};
use litellm_storage_clickhouse::fetch;
use litellm_traces_clickhouse::{
    content_reference::{self, ContentReference, Owner, REFERENCE_ATTRIBUTE},
    query::{
        lens::{
            LensAccessParams, LensContentParams, LensContentReferenceRow, LensContentReferences,
            LensContentReferencesParams, LensEvidenceParams,
        },
        named::SpanContentOwnerRow,
    },
};
use tokio::sync::Semaphore;

use crate::Error;

const MAX_CONCURRENT_READS: usize = 4;
const READ_TIMEOUT: Duration = Duration::from_secs(15);

type Connected = Box<dyn Fn() -> Option<(String, String)> + Send + Sync>;

#[derive(Clone)]
struct Endpoint {
    url: url::Url,
    token: Arc<str>,
}

fn content_url(base: &str) -> Option<url::Url> {
    format!("{}/lens/content", base.trim_end_matches('/')).parse().ok()
}

/// Reads proxy-generated trace content LiteLLM stored in object storage, through the gateway that holds the
/// storage credentials. Lens never holds them. The gateway is the static one when configured, else the one
/// connected at the time of the read.
#[derive(Clone)]
pub struct ContentGateway {
    client: reqwest::Client,
    fixed: Option<Endpoint>,
    connected: Arc<OnceLock<Connected>>,
    slots: Arc<Semaphore>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Hydrated {
    Content(BTreeMap<String, String>),
    Unavailable(String),
}

impl ContentGateway {
    /// `fixed` is a statically configured gateway, used only as a complete base and token pair.
    pub fn new(fixed: Option<(&url::Url, &str)>) -> Result<Self, Error> {
        let config = litellm_http::Resolution::from(&litellm_http::HttpSettings::default()).config;
        let client = reqwest::ClientBuilder::try_from(&config)?
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(READ_TIMEOUT)
            .build()?;
        Ok(Self {
            client,
            fixed: fixed.and_then(|(base, token)| {
                Some(Endpoint {
                    url: content_url(base.as_str())?,
                    token: token.into(),
                })
            }),
            connected: Arc::new(OnceLock::new()),
            slots: Arc::new(Semaphore::new(MAX_CONCURRENT_READS)),
        })
    }

    pub fn from_config(config: &crate::config::Config) -> Result<Self, Error> {
        let base = config.gateway_models.as_ref().map(|gateway| &gateway.base);
        Self::new(base.zip(config.gateway_service_token.as_deref()))
    }

    /// Reads the connected gateway's URL and service token on every use, so a gateway connected after
    /// startup is used without a restart.
    pub fn connected_by(
        &self,
        connected: impl Fn() -> Option<(String, String)> + Send + Sync + 'static,
    ) {
        let _ = self.connected.set(Box::new(connected));
    }

    fn endpoint(&self) -> Option<Endpoint> {
        self.fixed.clone().or_else(|| {
            let (base, token) = (self.connected.get()?)()?;
            Some(Endpoint {
                url: content_url(&base)?,
                token: token.into(),
            })
        })
    }

    /// What the reference says, verified for exactly this span before anything is fetched.
    fn verified(
        &self,
        owner: &Owner<'_>,
        raw: &str,
    ) -> Result<(Endpoint, ContentReference), Hydrated> {
        let endpoint = self
            .endpoint()
            .ok_or_else(|| Hydrated::Unavailable("retrieval_not_configured".into()))?;
        let key = content_reference::signing_key(&endpoint.token);
        let reference = content_reference::verify(&key, owner, raw)
            .ok_or_else(|| Hydrated::Unavailable("unverified".into()))?;
        Ok((endpoint, reference))
    }

    /// The span's stored content, or why it cannot be shown.
    pub async fn hydrate(&self, owner: &Owner<'_>, raw: &str) -> Hydrated {
        match self.verified(owner, raw) {
            Err(unavailable) => unavailable,
            Ok((_, ContentReference::Unavailable(reason))) => Hydrated::Unavailable(reason),
            Ok((endpoint, ContentReference::Stored { bytes, sha256, .. })) => self
                .download(&endpoint, owner, raw, bytes, &sha256)
                .await
                .unwrap_or_else(|reason| Hydrated::Unavailable(reason.into())),
        }
    }

    async fn download(
        &self,
        endpoint: &Endpoint,
        owner: &Owner<'_>,
        raw: &str,
        bytes: u64,
        sha256: &str,
    ) -> Result<Hydrated, &'static str> {
        let url = endpoint.url.clone();
        let _permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "retrieval_overloaded")?;
        let reference: serde_json::Value =
            serde_json::from_str(raw).map_err(|_| "retrieval_failed")?;
        let mut response = self
            .client
            .post(url)
            .bearer_auth(&*endpoint.token)
            .json(&serde_json::json!({
                "team_id": owner.team_id,
                "api_key": owner.api_key,
                "trace_id": owner.trace_id,
                "span_id": owner.span_id,
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
        serde_json::from_slice(&body)
            .map(Hydrated::Content)
            .map_err(|_| "integrity_failed")
    }
}

pub fn unavailable_text(reason: &str) -> String {
    format!("[content unavailable: {reason}]")
}

/// The span detail with its stored content read back and normalized exactly as an inline span's.
pub fn hydrated_detail(
    detail: SpanDetail,
    row: &SpanContentOwnerRow,
    hydrated: Hydrated,
) -> SpanDetail {
    let (input, output) = match hydrated {
        Hydrated::Content(content) => hydrated_input_output(
                &row.scope,
                &row.name,
                &row.parent_span_id,
                &with_content(&detail.attributes, content),
                &row.resource_attributes,
            )
            .unwrap_or_else(|_| (unavailable_text("unreadable"), String::new())),
        Hydrated::Unavailable(reason) => (unavailable_text(&reason), String::new()),
    };
    SpanDetail {
        input_ui: to_ui_content(&input),
        output_ui: to_ui_content(&output),
        input,
        output,
        ..detail
    }
}

fn with_content(
    attributes: &BTreeMap<String, String>,
    content: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    attributes
        .iter()
        .filter(|(key, _)| key.as_str() != REFERENCE_ATTRIBUTE)
        .map(|(key, value)| (key.clone(), value.clone()))
        .chain(content)
        .collect()
}

fn copied(access: &LensAccessParams) -> LensAccessParams {
    LensAccessParams {
        all_teams: access.all_teams,
        team: access.team.clone(),
        key_hash: access.key_hash.clone(),
    }
}

/// A referenced span's Input and Output, and whether they are its real content rather than a placeholder.
struct ReadBack {
    input: String,
    output: String,
    verifiable: bool,
}

fn proves(read: &ReadBack, row: &LensContentReferenceRow, quote: &str) -> bool {
    let (content, _) = content_reference::page(
        &read.input,
        &read.output,
        &row.status_code,
        &row.status_message,
        0,
    );
    read.verifiable && content.contains(quote)
}

impl super::Storage {
    async fn references(
        &self,
        parameters: LensContentReferencesParams,
    ) -> Vec<LensContentReferenceRow> {
        let reader = self.config.storage().reader();
        fetch::<LensContentReferences>(&self.client, reader, &parameters)
            .await
            .unwrap_or_default()
    }

    async fn read_back(&self, trace_id: &str, row: &LensContentReferenceRow) -> ReadBack {
        let raw = row
            .attributes
            .get(REFERENCE_ATTRIBUTE)
            .map(String::as_str)
            .unwrap_or_default();
        let owner = Owner {
            team_id: &row.team_id,
            api_key: &row.api_key,
            trace_id,
            span_id: &row.span_id,
        };
        let hydrated = match &self.content {
            Some(gateway) => gateway.hydrate(&owner, raw).await,
            None => Hydrated::Unavailable("retrieval_not_configured".into()),
        };
        let content = match hydrated {
            Hydrated::Content(content) => content,
            Hydrated::Unavailable(reason) => {
                return ReadBack {
                    input: unavailable_text(&reason),
                    output: String::new(),
                    verifiable: false,
                };
            }
        };
        match hydrated_input_output(
            &row.scope,
            &row.name,
            &row.parent_span_id,
            &with_content(&row.attributes, content),
            &row.resource_attributes,
        ) {
            Ok((input, output)) => ReadBack {
                input,
                output,
                verifiable: true,
            },
            Err(_) => ReadBack {
                input: unavailable_text("unreadable"),
                output: String::new(),
                verifiable: false,
            },
        }
    }

    /// The investigation page parts of the listed spans whose content LiteLLM stored, paged as inline content.
    pub(crate) async fn hydrated_parts(
        &self,
        parameters: &LensContentParams,
        span_ids: Vec<String>,
    ) -> Vec<(String, String, u8)> {
        let rows = self
            .references(LensContentReferencesParams {
                access: copied(&parameters.access),
                id: parameters.id.clone(),
                record_team: parameters.record_team.clone(),
                start_time: parameters.start_time.clone(),
                trace_ref: parameters.trace_ref.clone(),
                span_ids,
            })
            .await;
        let mut parts = Vec::with_capacity(rows.len());
        for row in &rows {
            let read = self.read_back(&parameters.id, row).await;
            let (content, truncated) = content_reference::page(
                &read.input,
                &read.output,
                &row.status_code,
                &row.status_message,
                parameters.offset,
            );
            parts.push((row.span_id.clone(), content, truncated));
        }
        parts
    }

    /// Whether the quote appears in the span's stored content; an unavailable placeholder never proves one.
    pub(crate) async fn hydrated_quote(&self, parameters: &LensEvidenceParams) -> bool {
        let rows = self
            .references(LensContentReferencesParams {
                access: copied(&parameters.access),
                id: parameters.id.clone(),
                record_team: parameters.record_team.clone(),
                start_time: parameters.start_time.clone(),
                trace_ref: parameters.trace_ref.clone(),
                span_ids: vec![parameters.span.clone()],
            })
            .await;
        for row in &rows {
            let read = self.read_back(&parameters.id, row).await;
            if proves(&read, row, &parameters.quote) {
                return true;
            }
        }
        false
    }
}

pub fn unavailable_detail(detail: SpanDetail, reason: &str) -> SpanDetail {
    let input = unavailable_text(reason);
    SpanDetail {
        input_ui: to_ui_content(&input),
        output_ui: to_ui_content(""),
        input,
        output: String::new(),
        ..detail
    }
}

pub fn owner<'a>(row: &'a SpanContentOwnerRow, trace_id: &'a str, span_id: &'a str) -> Owner<'a> {
    Owner {
        team_id: &row.team_id,
        api_key: &row.api_key,
        trace_id,
        span_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(attributes: &[(&str, &str)]) -> SpanDetail {
        SpanDetail {
            span_id: "b7ad6b7169203331".into(),
            input_ui: to_ui_content(""),
            output_ui: to_ui_content(""),
            input: String::new(),
            output: String::new(),
            attributes: attributes
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    fn row() -> SpanContentOwnerRow {
        SpanContentOwnerRow {
            team_id: "team-a".into(),
            api_key: "hash-a".into(),
            scope: "litellm".into(),
            name: "chat gpt-4o-mini".into(),
            parent_span_id: "parent".into(),
            resource_attributes: BTreeMap::from([("service.name".into(), "litellm".into())]),
            status_code: String::new(),
            status_message: String::new(),
        }
    }

    #[test]
    fn stored_content_is_shown_as_the_inline_span_would_show_it() {
        let content = BTreeMap::from([
            (
                "gen_ai.input.messages".to_owned(),
                r#"[{"role": "user", "content": "list the files"}]"#.to_owned(),
            ),
            (
                "gen_ai.output.messages".to_owned(),
                r#"[{"role": "assistant", "content": "one file"}]"#.to_owned(),
            ),
        ]);
        let metadata = [("gen_ai.operation.name", "chat"), (REFERENCE_ATTRIBUTE, "{}")];
        let inline_attributes: BTreeMap<String, String> = detail(&metadata[..1])
            .attributes
            .into_iter()
            .chain(content.clone())
            .collect();
        let expected = hydrated_input_output(
            "litellm",
            "chat gpt-4o-mini",
            "parent",
            &inline_attributes,
            &row().resource_attributes,
        )
        .unwrap();
        let shown = hydrated_detail(detail(&metadata), &row(), Hydrated::Content(content));
        assert_eq!((shown.input.clone(), shown.output.clone()), expected);
        assert_eq!(shown.input_ui, to_ui_content(&expected.0));
        assert!(shown.input.contains("list the files") && shown.output.contains("one file"));
    }

    #[test]
    fn unavailable_content_is_shown_as_a_placeholder_and_keeps_the_span_metadata() {
        let shown = hydrated_detail(
            detail(&[("gen_ai.operation.name", "chat")]),
            &row(),
            Hydrated::Unavailable("upload_failed".into()),
        );
        assert_eq!(shown.input, "[content unavailable: upload_failed]");
        assert_eq!(shown.attributes["gen_ai.operation.name"], "chat");
    }

    const TOKEN: &str = "fixture-service-token-with-32-chars!";
    const STORED: &str = r#"{"v":2,"state":"stored","object_key":"logs/lens-content/2023-11-14/abc.json","bytes":45,"sha256":"0e469198a46d22437fab286514c9c8a42c17552d50e8613225e2c31bd5407968","mac":"c2172cc69bbddfc0003ccea9602142710e9f4e3a9a639f62d18715f667ce0a0f"}"#;
    const UNAVAILABLE: &str = r#"{"v":2,"state":"unavailable","reason":"upload_failed","mac":"b691bdbb706af2a54e56f68077a6a63c909d5e6248aa37c1764feff4d88f30b9"}"#;

    fn fixture_owner() -> Owner<'static> {
        Owner {
            team_id: "team-\u{e9}",
            api_key: "hash-a",
            trace_id: "0af7651916cd43dd8448eb211c80319c",
            span_id: "b7ad6b7169203331",
        }
    }

    fn read_url(gateway: &ContentGateway, raw: &str) -> Result<String, Hydrated> {
        gateway
            .verified(&fixture_owner(), raw)
            .map(|(endpoint, _)| String::from(endpoint.url))
    }

    #[test]
    fn a_gateway_connected_after_startup_is_used_with_its_current_token() {
        let connected = Arc::new(std::sync::RwLock::new(None::<(String, String)>));
        let gateway = ContentGateway::new(None).unwrap();
        let shared = connected.clone();
        gateway.connected_by(move || shared.read().unwrap().clone());
        assert_eq!(
            read_url(&gateway, UNAVAILABLE),
            Err(Hydrated::Unavailable("retrieval_not_configured".into()))
        );
        *connected.write().unwrap() = Some(("http://gateway.test/".into(), TOKEN.into()));
        assert_eq!(
            read_url(&gateway, UNAVAILABLE).unwrap(),
            "http://gateway.test/lens/content"
        );
        assert!(matches!(
            gateway.verified(&fixture_owner(), UNAVAILABLE),
            Ok((_, ContentReference::Unavailable(reason))) if reason == "upload_failed"
        ));
        *connected.write().unwrap() =
            Some(("http://gateway.test/".into(), "a-rotated-service-token-32-characters".into()));
        assert_eq!(
            read_url(&gateway, UNAVAILABLE),
            Err(Hydrated::Unavailable("unverified".into()))
        );
    }

    #[test]
    fn a_static_token_without_a_static_gateway_reads_through_the_connected_gateway() {
        let config = crate::config::Config::read(|name| match name {
            "LENS_ADMIN_TOKEN" => Some("standalone-setup-token-at-least-32-characters".into()),
            "LITELLM_LENS_SERVICE_TOKEN" => Some(TOKEN.into()),
            _ => None,
        })
        .unwrap();
        assert!(config.gateway_models.is_none());
        let gateway = ContentGateway::from_config(&config).unwrap();
        assert_eq!(
            read_url(&gateway, UNAVAILABLE),
            Err(Hydrated::Unavailable("retrieval_not_configured".into()))
        );
        gateway.connected_by(|| Some(("http://gateway.test".into(), TOKEN.into())));
        assert_eq!(
            read_url(&gateway, STORED).unwrap(),
            "http://gateway.test/lens/content"
        );
        assert!(matches!(
            gateway.verified(&fixture_owner(), UNAVAILABLE),
            Ok((_, ContentReference::Unavailable(reason))) if reason == "upload_failed"
        ));
    }

    #[test]
    fn a_statically_configured_gateway_wins_over_a_connected_one() {
        let base: url::Url = "http://static.test".parse().unwrap();
        let gateway = ContentGateway::new(Some((&base, TOKEN))).unwrap();
        gateway.connected_by(|| {
            Some((
                "http://other.test".into(),
                "another-token-with-32-characters!!".into(),
            ))
        });
        assert_eq!(
            read_url(&gateway, STORED).unwrap(),
            "http://static.test/lens/content"
        );
    }

    #[test]
    fn a_reference_that_does_not_verify_is_never_fetched() {
        let base: url::Url = "http://static.test".parse().unwrap();
        let gateway = ContentGateway::new(Some((&base, TOKEN))).unwrap();
        let tampered = STORED.replace("\"bytes\":45", "\"bytes\":46");
        let reasoned = UNAVAILABLE.replace("upload_failed", "too_large");
        let refused = [
            "",
            "{",
            &STORED[..60],
            tampered.as_str(),
            reasoned.as_str(),
        ];
        for raw in refused {
            assert_eq!(
                read_url(&gateway, raw),
                Err(Hydrated::Unavailable("unverified".into()))
            );
        }
    }

    #[test]
    fn an_unavailable_placeholder_never_proves_a_quote() {
        let row = LensContentReferenceRow {
            span_id: "b7ad6b7169203331".into(),
            team_id: "team-a".into(),
            api_key: "hash-a".into(),
            scope: "litellm".into(),
            name: "chat".into(),
            parent_span_id: String::new(),
            resource_attributes: BTreeMap::new(),
            status_code: "Unset".into(),
            status_message: String::new(),
            attributes: BTreeMap::new(),
        };
        let placeholder = ReadBack {
            input: unavailable_text("upload_failed"),
            output: String::new(),
            verifiable: false,
        };
        assert!(!proves(&placeholder, &row, "content unavailable"));
        let content = ReadBack {
            input: "list the files".into(),
            output: "one file".into(),
            verifiable: true,
        };
        assert!(proves(&content, &row, "one file"));
        assert!(!proves(&content, &row, "two files"));
    }
}
