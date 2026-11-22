use std::{collections::BTreeMap, sync::Arc, time::Duration};

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

/// Reads proxy-generated trace content LiteLLM stored in object storage, through the gateway that holds the
/// storage credentials. Lens never holds them.
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
    Content(BTreeMap<String, String>),
    Unavailable(String),
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

    /// The span's stored content, or why it cannot be shown; None when the reference does not verify for it.
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
            Some(gateway) => gateway
                .hydrate(&owner, raw)
                .await
                .unwrap_or_else(|| Hydrated::Unavailable("unverified".into())),
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
            let (content, _) = content_reference::page(
                &read.input,
                &read.output,
                &row.status_code,
                &row.status_message,
                0,
            );
            if read.verifiable && content.contains(parameters.quote.as_str()) {
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
}
