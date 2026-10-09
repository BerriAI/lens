use lens_contract::signals::{NoulAnswer, SignalStep};
use lens_decisions::{Deployment, EvaluationModels, Provider, Secret, TransportLimits};
use lens_signals::{DecisionRequest, Question, SignalState};
use litellm_model_catalog::Catalog;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider: Provider = serde_json::from_value(serde_json::Value::String(std::env::var(
        "LENS_EVALUATION_PROVIDER",
    )?))?;
    let model = std::env::var("LENS_EVALUATION_MODEL")?;
    let models = EvaluationModels::new(
        Arc::new(Catalog::parse(
            br#"{"evaluation-fixture":{"mode":"evaluation","litellm_provider":"typesafe"}}"#,
            Default::default(),
        )?),
        vec![Deployment {
            name: "qualification".into(),
            model: model.clone(),
            provider,
            api_base: std::env::var("LENS_PROVIDER_API_BASE")
                .ok()
                .map(|base| base.parse())
                .transpose()?,
            api_key: std::env::var("LENS_PROVIDER_API_KEY").ok().map(Secret::new),
        }],
        TransportLimits {
            timeout: Duration::from_secs(60),
            max_response_bytes: 65536,
        },
    )?;
    let request=DecisionRequest{model:"qualification".into(),state:SignalState{task:"An AI agent run recorded as a trace. Judge only what the user and the agent said and did in these steps.",steps:vec![SignalStep{kind:"user".into(),name:"message".into(),content:"I am frustrated. You have ignored my request three times. Please stop repeating the same answer.".into()}]},questions:BTreeMap::from([("frustration".into(),Question{r#type:"noul",instructions:"Does the user express frustration with the agent?".into()})]),timeout:Duration::from_secs(60),tags:vec!["litellm-lens-signals"]};
    let response = models.evaluate(&request).await?;
    let NoulAnswer::Noul { noul } =
        serde_json::from_value(response["answers"]["frustration"].clone())?;
    println!(
        "{}",
        serde_json::json!({"model":model,"frustration_score":noul})
    );
    Ok(())
}
