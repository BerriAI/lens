#![forbid(unsafe_code)]

use lens_analysis::{
    AnalysisModels, Deployment, Provider, Secret, TransportLimits, bundled_catalog,
};
use lens_contract::worker::ModelRequest;
use lens_inference::{ModelCapacity, OutputLimits};
use serde_json::json;
use std::{env, num::NonZeroU64, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = match env::var("LENS_PROVIDER")?.as_str() {
        "openai" => Provider::OpenAi,
        "anthropic" => Provider::Anthropic,
        "openai_compatible" => Provider::OpenAiCompatible,
        _ => return Err("LENS_PROVIDER must be openai, anthropic or openai_compatible".into()),
    };
    let config = Deployment {
        name: "check".into(),
        model: env::var("LENS_ANALYSIS_MODEL")?,
        provider,
        api_base: env::var("LENS_PROVIDER_API_BASE")
            .ok()
            .map(|base| base.parse())
            .transpose()?,
        api_key: Secret::new(env::var("LENS_PROVIDER_API_KEY")?),
        input_cost_per_token: None,
        output_cost_per_token: None,
        capacity: ModelCapacity::default(),
        output_limits: OutputLimits {
            max_tokens: NonZeroU64::new(512),
            ..OutputLimits::default()
        },
    };
    let models = AnalysisModels::new(
        bundled_catalog()?,
        vec![config],
        TransportLimits {
            timeout: Duration::from_secs(60),
            max_response_bytes: 65536,
        },
    )?;
    let body: ModelRequest = serde_json::from_value(
        json!({"prompt":"Check transport", "purpose":"extract", "messages":[{"role":"user","content":"Return exactly the JSON object {\"ok\":true}."}]}),
    )?;
    let prepared = models.prepare("check", &body).await?;
    let completion = models.complete(&prepared).await?;
    let result: serde_json::Value = serde_json::from_str(&completion.result.content)?;
    if result != json!({"ok":true}) || completion.result.context_exceeded {
        return Err("Provider check returned unexpected content".into());
    }
    let usage = completion.usage.usage.unwrap_or_default();
    println!(
        "{}",
        json!({"ok":true,"model":completion.usage.model,"prompt_tokens":usage.prompt_tokens,"completion_tokens":usage.completion_tokens,"cost":completion.result.cost,"estimate":prepared.estimate})
    );
    Ok(())
}
