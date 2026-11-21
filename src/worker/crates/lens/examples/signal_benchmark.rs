use std::{env, error::Error, fs, sync::Arc};

use lens_analysis::{AnalysisModels, Deployment, Provider, Secret, bundled_catalog};
use lens_contract::{
    investigations::Scope,
    signals::{Signal, SignalConfig},
    worker::{Execution, ExecutionContent, Sample},
};
use lens_decisions::EvaluationModels;
use lens_signals::{SignalReader, SourceError, classify};
use litellm_lens::signals::SignalModels;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Suite {
    signals: Vec<Signal>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    split: String,
    content: Vec<String>,
    expected: Vec<String>,
}

struct Trace(Vec<String>);

impl SignalReader for Trace {
    async fn content(
        &self,
        _: &Scope,
        execution: &Execution,
        _: &str,
    ) -> Result<ExecutionContent, SourceError> {
        Ok(serde_json::from_value(json!({
            "execution": execution,
            "parts": self.0.iter().enumerate().map(|(i, content)| json!({
                "content": content, "kind": "llm", "name": "recorded step",
                "execution_id": execution.id, "span_id": format!("s{i}"),
                "start_time": format!("{i:04}")
            })).collect::<Vec<_>>()
        }))
        .unwrap())
    }

    async fn sample(
        &self,
        _: &Scope,
        _: i64,
        _: i64,
        _: u32,
        _: &str,
    ) -> Result<Sample, SourceError> {
        unreachable!("benchmark passes each fixture directly to the classifier")
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let file = args.next().ok_or("expected a benchmark fixture path")?;
    let split = args.next().ok_or("expected a fixture split")?;
    let model = env::var("BENCHMARK_MODEL").unwrap_or_else(|_| "gpt-6.2-sol".into());
    let suite: Suite = serde_json::from_str(&fs::read_to_string(file)?)?;
    if !suite.cases.iter().any(|case| case.split == split) {
        return Err("no cases for the requested split".into());
    }
    let catalog = bundled_catalog()?;
    let tested_judge = model == "gpt-6.2-sol";
    let models = SignalModels {
        evaluation: Arc::new(EvaluationModels::new(
            catalog.clone(),
            vec![],
            Default::default(),
        )?),
        analysis: Arc::new(AnalysisModels::new(
            catalog,
            vec![Deployment {
                name: "judge".into(),
                model: model.clone(),
                provider: Provider::OpenAi,
                api_base: None,
                api_key: Secret::new(env::var("BENCHMARK_API_KEY")?),
                input_cost_per_token: tested_judge.then_some(0.000002),
                output_cost_per_token: tested_judge.then_some(0.00001),
                capacity: lens_inference::ModelCapacity {
                    max_input_tokens: tested_judge.then(|| 922_000.try_into().unwrap()),
                    max_output_tokens: tested_judge.then(|| 128_000.try_into().unwrap()),
                },
                output_limits: Default::default(),
            }],
            Default::default(),
        )?),
    };
    let config = SignalConfig {
        model: "judge".into(),
        threshold: 0.5,
        signals: suite.signals,
    };
    let execution: Execution = serde_json::from_value(json!({
        "id":"benchmark", "source":"traces", "name":"Benchmark", "trace_id":"benchmark",
        "trace_ref":"", "team_id":"", "start_time":"", "span_count":1, "root_seen":true,
        "service":"", "metadata":[]
    }))?;
    let cases = suite
        .cases
        .into_iter()
        .enumerate()
        .filter(|(_, case)| case.split == split);
    let tasks = futures_util::stream::iter(cases.map(|(i, case)| {
        let model = &model;
        let models = &models;
        let config = &config;
        let execution = &execution;
        async move {
            let trace = Trace(case.content);
            let result = classify(&trace, models, &Scope::default(), execution, config).await;
            let valid = result.evidence.values().all(|evidence| {
                evidence
                    .span_id
                    .strip_prefix('s')
                    .and_then(|index| index.parse::<usize>().ok())
                    .and_then(|index| trace.0.get(index))
                    .is_some_and(|step| step.contains(&evidence.quote))
            });
            println!(
                "{}",
                json!({
                    "case": i + 1, "model": model, "threshold": config.threshold,
                    "expected": case.expected, "result": result, "evidence_valid": valid
                })
            );
        }
    }));
    use futures_util::StreamExt;
    tasks.buffer_unordered(6).for_each(|()| async {}).await;
    Ok(())
}
