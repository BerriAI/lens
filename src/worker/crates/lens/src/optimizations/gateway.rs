use crate::{error::OptimizationError as Error, gateway::GatewayConfig};
use lens_contract::optimizations::ReplayRequest;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Clone)]
pub(super) struct Gateway {
    config: GatewayConfig,
    identity: Option<lens_inference::GatewayIdentity>,
    client: reqwest::Client,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Model {
    pub model_group: String,
    pub mode: Option<String>,
    pub input_cost_per_token: Option<f64>,
    pub output_cost_per_token: Option<f64>,
    pub supported_openai_params: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct Catalog {
    data: Vec<Model>,
}
#[derive(Deserialize)]
struct Completion {
    model: String,
    choices: Vec<Choice>,
}
#[derive(Deserialize)]
struct Choice {
    finish_reason: Option<String>,
    message: Value,
}

pub(super) struct Reply {
    pub model: String,
    pub output: String,
    pub cost: Option<f64>,
    pub duration_ms: u64,
    pub error: Option<&'static str>,
}

pub(super) struct JudgeEvidence<'a> {
    pub model: &'a str,
    pub rubric: &'a str,
    pub input: &'a str,
    pub expected: &'a str,
    pub context: &'a str,
    pub output: &'a str,
    pub run: &'a str,
    pub case: &'a str,
}

impl Gateway {
    pub fn new(
        config: GatewayConfig,
        identity: Option<lens_inference::GatewayIdentity>,
    ) -> Result<Self, Error> {
        Ok(Self {
            config,
            identity,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(180))
                .build()
                .map_err(|_| Error::Gateway)?,
        })
    }

    async fn request<T: DeserializeOwned>(
        &self,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(T, Option<f64>), Error> {
        let url = format!(
            "{}/{}",
            self.config.base.as_str().trim_end_matches('/'),
            path
        );
        let endpoint = url::Url::parse(&url).map_err(|_| Error::Gateway)?;
        let request = match body {
            Some(body) => self.client.post(endpoint.clone()).json(body),
            None => self.client.get(endpoint.clone()),
        }
        .bearer_auth(&self.config.key);
        let request = match &self.identity {
            Some(identity) => match identity
                .token(
                    &endpoint,
                    lens_inference::GatewayPurpose::Analysis,
                    chrono::Utc::now(),
                )
                .map_err(|_| Error::Gateway)?
            {
                Some(token) => request.header(lens_inference::GATEWAY_HEADER, token),
                None => return Err(Error::Gateway),
            },
            None => request,
        };
        let mut response = request.send().await.map_err(|_| Error::Gateway)?;
        if !response.status().is_success() {
            return Err(Error::Gateway);
        }
        let cost = response
            .headers()
            .get("x-litellm-response-cost")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|cost| cost.is_finite() && *cost >= 0.0);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Gateway)? {
            if bytes.len().saturating_add(chunk.len()) > 8_000_000 {
                return Err(Error::Gateway);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((
            serde_json::from_slice(&bytes).map_err(|_| Error::Gateway)?,
            cost,
        ))
    }

    pub async fn models(&self) -> Result<Vec<Model>, Error> {
        let (catalog, _) = self.request::<Catalog>("model_group/info", None).await?;
        Ok(catalog
            .data
            .into_iter()
            .filter(|model| model.mode.as_deref() == Some("chat"))
            .collect())
    }

    pub async fn complete(
        &self,
        model: &str,
        request: &ReplayRequest,
        run: &str,
        case: &str,
    ) -> Result<Reply, Error> {
        request.validate().map_err(Error::Invalid)?;
        let mut body = request.parameters.clone();
        body.insert("messages".into(), json!(request.messages));
        body.insert("model".into(), json!(model));
        body.insert("stream".into(), json!(false));
        body.insert("metadata".into(), json!({"litellm_lens_internal":true,"deployment.environment":"lens-eval","lens_optimization_id":run,"lens_case_id":case}));
        let start = std::time::Instant::now();
        let (response, cost) = self
            .request::<Completion>("chat/completions", Some(&Value::Object(body)))
            .await?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or(Error::Invalid("The model returned no answer"))?;
        let error = if response.model != model {
            Some("The gateway served a different model; this result is excluded")
        } else if choice.finish_reason.as_deref() != Some("stop")
            || choice
                .message
                .get("tool_calls")
                .is_some_and(|calls| calls.as_array().is_some_and(|calls| !calls.is_empty()))
        {
            Some("The model did not produce a complete final answer")
        } else if choice
            .message
            .get("content")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            Some("The model returned an empty answer")
        } else {
            None
        };
        let output = choice
            .message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(Reply {
            error,
            model: response.model,
            output,
            cost,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    pub async fn judge(&self, evidence: JudgeEvidence<'_>) -> Result<f64, Error> {
        let content = json!({"rubric":evidence.rubric,"input":evidence.input,"expected":evidence.expected,"recorded_context":evidence.context,"output":evidence.output});
        let request = ReplayRequest {
            messages: vec![
                json!({"role":"system","content":crate::eval_judge::INSTRUCTIONS}),
                json!({"role":"user","content":content.to_string()}),
            ],
            parameters: serde_json::Map::from_iter([(
                "response_format".into(),
                json!({"type":"json_object"}),
            )]),
        };
        let reply = self
            .complete(evidence.model, &request, evidence.run, evidence.case)
            .await?;
        if let Some(error) = reply.error {
            return Err(Error::Invalid(error));
        }
        let score: crate::eval_judge::Score = serde_json::from_str(&reply.output)
            .map_err(|_| Error::Invalid("The judge returned an invalid score"))?;
        if !(0.0..=1.0).contains(&score.score) {
            return Err(Error::Invalid("The judge returned an invalid score"));
        }
        Ok(score.score)
    }
}
