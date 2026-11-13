use std::time::Duration;

use lens_contract::profiles::ModelCapabilities;
use lens_server::profiles::ConfiguredModel;
use serde::{Deserialize, Serialize};

use crate::{error::EmbedError, gateway::GatewayConfig};

pub const EMBED_BATCH: usize = 20;
const TIMEOUT: Duration = Duration::from_secs(60);
pub const INTERNAL_TAG: &str = "litellm-lens-profiles";

#[derive(Deserialize)]
struct ModelInfo {
    data: Vec<Deployment>,
}

#[derive(Deserialize)]
struct Deployment {
    model_name: String,
    #[serde(default)]
    model_info: Info,
}

#[derive(Default, Deserialize)]
struct Info {
    id: Option<String>,
    mode: Option<String>,
    litellm_provider: Option<String>,
    supports_function_calling: Option<bool>,
    supports_vision: Option<bool>,
    supports_reasoning: Option<bool>,
    supports_response_schema: Option<bool>,
}

fn configured(info: ModelInfo) -> Vec<ConfiguredModel> {
    info.data
        .into_iter()
        .filter(|deployment| {
            deployment
                .model_info
                .mode
                .as_deref()
                .is_none_or(|mode| matches!(mode, "chat" | "responses" | "completion"))
        })
        .filter_map(|deployment| {
            Some(ConfiguredModel {
                model_id: deployment.model_info.id.filter(|id| !id.is_empty())?,
                model_group: deployment.model_name,
                provider: deployment.model_info.litellm_provider.unwrap_or_default(),
                capabilities: ModelCapabilities {
                    function_calling: deployment.model_info.supports_function_calling,
                    vision: deployment.model_info.supports_vision,
                    reasoning: deployment.model_info.supports_reasoning,
                    structured_output: deployment.model_info.supports_response_schema,
                },
            })
        })
        .collect()
}

/// The gateway's configured chat deployments with the capability flags it declares. A flag the
/// gateway leaves out stays unknown.
pub async fn configured_models(
    client: &reqwest::Client,
    connection: &GatewayConfig,
) -> Option<Vec<ConfiguredModel>> {
    let response = client
        .get(connection.endpoint("/model/info"))
        .bearer_auth(connection.key())
        .timeout(TIMEOUT)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    response.json::<ModelInfo>().await.ok().map(configured)
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
    metadata: InternalMetadata,
}

#[derive(Serialize)]
pub struct InternalMetadata {
    pub litellm_lens_internal: bool,
    pub tags: [&'static str; 1],
}

impl InternalMetadata {
    pub fn new() -> Self {
        Self {
            litellm_lens_internal: true,
            tags: [INTERNAL_TAG],
        }
    }
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<Embedding>,
}

#[derive(Deserialize)]
struct Embedding {
    index: usize,
    embedding: Vec<f32>,
}

fn ordered(reply: EmbeddingResponse, expected: usize) -> Result<Vec<Vec<f32>>, EmbedError> {
    let mut data = reply.data;
    data.sort_by_key(|item| item.index);
    let valid = data.len() == expected
        && data
            .iter()
            .enumerate()
            .all(|(position, item)| item.index == position)
        && data.iter().all(|item| {
            !item.embedding.is_empty()
                && item.embedding.len() == data[0].embedding.len()
                && item.embedding.iter().all(|value| value.is_finite())
        });
    if !valid {
        return Err(EmbedError::InvalidReply);
    }
    Ok(data.into_iter().map(|item| item.embedding).collect())
}

/// Embeds through the gateway's OpenAI-compatible endpoint in batches of at most 20, the
/// largest batch every supported embedding provider accepts.
pub async fn embed(
    client: &reqwest::Client,
    connection: &GatewayConfig,
    model: &str,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, EmbedError> {
    let mut embeddings = Vec::with_capacity(texts.len());
    for batch in texts.chunks(EMBED_BATCH) {
        let response = client
            .post(connection.endpoint("/v1/embeddings"))
            .bearer_auth(connection.key())
            .timeout(TIMEOUT)
            .json(&EmbeddingRequest {
                model,
                input: batch,
                metadata: InternalMetadata::new(),
            })
            .send()
            .await
            .map_err(|_| EmbedError::Connection)?;
        if !response.status().is_success() {
            return Err(EmbedError::Rejected(response.status().as_u16()));
        }
        let reply = response
            .json::<EmbeddingResponse>()
            .await
            .map_err(|_| EmbedError::InvalidReply)?;
        embeddings.extend(ordered(reply, batch.len())?);
    }
    Ok(embeddings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, header, method, path},
    };

    #[rstest]
    fn undeclared_capabilities_stay_unknown_and_embedding_deployments_are_not_columns() {
        let info: ModelInfo = serde_json::from_value(json!({"data": [
            {"model_name": "sonnet", "model_info": {"id": "dep-1", "mode": "chat", "litellm_provider": "anthropic",
                "supports_vision": true, "supports_function_calling": false}},
            {"model_name": "qwen-embed", "model_info": {"id": "dep-2", "mode": "embedding"}},
            {"model_name": "no-id", "model_info": {"mode": "chat"}}
        ]}))
        .expect("model info");

        let models = configured(info);

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "dep-1");
        assert_eq!(models[0].capabilities.vision, Some(true));
        assert_eq!(models[0].capabilities.function_calling, Some(false));
        assert_eq!(models[0].capabilities.reasoning, None);
    }

    #[rstest]
    fn embeddings_are_reordered_by_index() {
        let reply: EmbeddingResponse = serde_json::from_value(json!({"data": [
            {"index": 1, "embedding": [0.0, 1.0]},
            {"index": 0, "embedding": [1.0, 0.0]}
        ]}))
        .expect("reply");

        assert_eq!(ordered(reply, 2), Ok(vec![vec![1.0, 0.0], vec![0.0, 1.0]]));
    }

    fn texts(count: usize) -> Vec<String> {
        (0..count).map(|index| format!("request {index}")).collect()
    }

    fn vectors(batch: usize, offset: usize) -> serde_json::Value {
        json!({"data": (0..batch).rev().map(|index| json!({"index": index, "embedding": [(offset + index) as f32, 1.0]})).collect::<Vec<_>>()})
    }

    #[rstest]
    #[tokio::test]
    async fn embeddings_are_requested_in_tagged_batches_and_returned_in_input_order() {
        let gateway = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(header("authorization", "Bearer fixture-key"))
            .and(body_partial_json(json!({"model": "qwen", "input": texts(EMBED_BATCH), "metadata": {"litellm_lens_internal": true, "tags": [INTERNAL_TAG]}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(vectors(EMBED_BATCH, 0)))
            .expect(1)
            .mount(&gateway)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(body_partial_json(
                json!({"input": ["request 20", "request 21"]}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(vectors(2, EMBED_BATCH)))
            .expect(1)
            .mount(&gateway)
            .await;
        let connection =
            GatewayConfig::new(&gateway.uri(), "fixture-key".into()).expect("connection");

        let embedded = embed(
            &reqwest::Client::new(),
            &connection,
            "qwen",
            &texts(EMBED_BATCH + 2),
        )
        .await
        .expect("embeddings");

        assert_eq!(embedded.len(), EMBED_BATCH + 2);
        assert_eq!(embedded[0], vec![0.0, 1.0]);
        assert_eq!(embedded[EMBED_BATCH + 1], vec![21.0, 1.0]);
    }

    #[rstest]
    #[case::rejected(ResponseTemplate::new(429), EmbedError::Rejected(429))]
    #[case::not_json(ResponseTemplate::new(200).set_body_string("<html>"), EmbedError::InvalidReply)]
    #[tokio::test]
    async fn a_failed_embedding_call_fails_discovery_with_its_reason(
        #[case] reply: ResponseTemplate,
        #[case] expected: EmbedError,
    ) {
        let gateway = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(reply)
            .mount(&gateway)
            .await;
        let connection =
            GatewayConfig::new(&gateway.uri(), "fixture-key".into()).expect("connection");

        let result = embed(&reqwest::Client::new(), &connection, "qwen", &texts(2)).await;

        assert_eq!(result, Err(expected));
    }

    #[rstest]
    #[tokio::test]
    async fn an_unreachable_gateway_is_a_connection_error() {
        let gateway = MockServer::start().await;
        let connection =
            GatewayConfig::new(&gateway.uri(), "fixture-key".into()).expect("connection");
        drop(gateway);

        let result = embed(&reqwest::Client::new(), &connection, "qwen", &texts(1)).await;

        assert_eq!(result, Err(EmbedError::Connection));
    }

    #[rstest]
    #[case::listed(ResponseTemplate::new(200).set_body_json(json!({"data": [{"model_name": "sonnet", "model_info": {"id": "dep-1", "mode": "chat"}}]})), Some(1))]
    #[case::rejected(ResponseTemplate::new(401), None)]
    #[case::not_json(ResponseTemplate::new(200).set_body_string("nope"), None)]
    #[tokio::test]
    async fn configured_models_come_from_the_gateway_model_list(
        #[case] reply: ResponseTemplate,
        #[case] expected: Option<usize>,
    ) {
        let gateway = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/model/info"))
            .and(header("authorization", "Bearer fixture-key"))
            .respond_with(reply)
            .mount(&gateway)
            .await;
        let connection =
            GatewayConfig::new(&gateway.uri(), "fixture-key".into()).expect("connection");

        let models = configured_models(&reqwest::Client::new(), &connection).await;

        assert_eq!(models.map(|models| models.len()), expected);
    }

    #[rstest]
    #[case::missing_item(json!({"data": [{"index": 0, "embedding": [1.0]}]}))]
    #[case::ragged(json!({"data": [{"index": 0, "embedding": [1.0]}, {"index": 1, "embedding": [1.0, 2.0]}]}))]
    #[case::duplicate_index(json!({"data": [{"index": 0, "embedding": [1.0]}, {"index": 0, "embedding": [2.0]}]}))]
    fn malformed_embedding_replies_are_rejected(#[case] reply: serde_json::Value) {
        let reply: EmbeddingResponse = serde_json::from_value(reply).expect("reply");

        assert_eq!(ordered(reply, 2), Err(EmbedError::InvalidReply));
    }
}
