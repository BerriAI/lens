use std::time::Duration;

use lens_contract::profiles::ModelCapabilities;
use lens_server::profiles::ConfiguredModel;
use serde::Deserialize;

use crate::gateway::GatewayConfig;

const TIMEOUT: Duration = Duration::from_secs(60);

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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
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
}
