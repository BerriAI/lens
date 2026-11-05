#[path = "investigation_bench/fixtures.rs"]
mod fixtures;
#[path = "investigation_bench/load.rs"]
mod load;

use fixtures::Fixture;
use futures_util::future::BoxFuture;
use lens_analysis::{AnalysisModels, Deployment, Provider, Secret, TransportLimits};
use litellm_lens::{
    Error,
    control::{JobBackend, JobClient},
    pipeline, wire,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Deserialize, Serialize)]
struct Recorded {
    request: wire::ModelRequest,
    result: wire::ModelResult,
    prompt_tokens: i64,
    completion_tokens: i64,
    elapsed_ms: u64,
}

struct Metrics {
    stage: String,
    since: Instant,
    phases_ms: BTreeMap<String, u64>,
    calls: Vec<Value>,
    failures: Vec<Value>,
    pages: u64,
    reserved: f64,
    cost: f64,
    attempts: usize,
    reviews: BTreeMap<String, wire::Review>,
}

struct Backend {
    fixture: Fixture,
    models: Option<AnalysisModels>,
    model: String,
    mode: String,
    tape: PathBuf,
    budget: f64,
    call_limit: usize,
    io_delay: Duration,
    metrics: Mutex<Metrics>,
}

impl Backend {
    async fn infer(&self, request: &wire::ModelRequest) -> Result<wire::ModelResult, Error> {
        let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(request)?));
        let path = self.tape.join(format!("{hash}.json"));
        {
            let mut metrics = self.metrics.lock().unwrap();
            metrics.attempts += 1;
            if metrics.attempts > self.call_limit {
                return Err(Error::InvalidRequest);
            }
        }
        let started = Instant::now();
        let recorded = if self.mode == "replay" {
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|_| Error::InvalidRequest)?;
            let record: Recorded = serde_json::from_slice(&bytes)?;
            if serde_json::to_vec(&record.request)? != serde_json::to_vec(request)? {
                return Err(Error::InvalidRequest);
            }
            tokio::time::sleep(Duration::from_millis(record.elapsed_ms)).await;
            record
        } else {
            let models = self.models.as_ref().ok_or(Error::InvalidRequest)?;
            let prepared = models.prepare(&self.model, request).await?;
            {
                let mut metrics = self.metrics.lock().unwrap();
                if metrics.cost + metrics.reserved + prepared.estimate > self.budget {
                    return Err(Error::InvalidRequest);
                }
                metrics.reserved += prepared.estimate;
            }
            let response = models.complete(&prepared).await;
            let mut metrics = self.metrics.lock().unwrap();
            metrics.reserved -= prepared.estimate;
            if let Err(error) = &response {
                let failure = json!({"request":request,"request_sha256":hash,"error":error.to_string(),"elapsed_ms":started.elapsed().as_millis(),"estimated_cost_usd":prepared.estimate,"actual_cost_usd":null});
                std::fs::write(
                    path.with_extension("failed.json"),
                    serde_json::to_vec(&failure)?,
                )?;
                metrics.failures.push(failure);
            }
            let completion = response?;
            metrics.cost += completion.result.cost;
            let usage = completion.usage.usage.unwrap_or_default();
            Recorded {
                request: request.clone(),
                result: completion.result,
                prompt_tokens: usage.prompt_tokens.unwrap_or_default(),
                completion_tokens: usage.completion_tokens.unwrap_or_default(),
                elapsed_ms: started.elapsed().as_millis() as u64,
            }
        };
        self.metrics.lock().unwrap().calls.push(json!({"purpose":request.purpose,"request_sha256":hash,"elapsed_ms":started.elapsed().as_millis(),"recorded_inference_ms":recorded.elapsed_ms,"prompt_tokens":recorded.prompt_tokens,"completion_tokens":recorded.completion_tokens,"catalog_cost_usd":recorded.result.cost}));
        if self.mode == "record" {
            tokio::fs::write(path, serde_json::to_vec(&recorded)?)
                .await
                .map_err(|_| Error::InvalidRequest)?;
        }
        Ok(recorded.result)
    }
}

impl JobBackend for Backend {
    fn sample(&self) -> BoxFuture<'_, Result<wire::Sample, Error>> {
        Box::pin(async { Ok(self.fixture.sample.clone()) })
    }
    fn reviews(&self) -> BoxFuture<'_, Result<Vec<wire::Review>, Error>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn content<'a>(
        &'a self,
        id: &'a str,
        cursor: &'a str,
        offset: usize,
    ) -> BoxFuture<'a, Result<wire::ExecutionContent, Error>> {
        Box::pin(async move {
            self.metrics.lock().unwrap().pages += 1;
            if !self.io_delay.is_zero() {
                tokio::time::sleep(self.io_delay).await;
            }
            self.fixture.content(id, cursor, offset)
        })
    }
    fn model<'a>(
        &'a self,
        request: &'a wire::ModelRequest,
    ) -> BoxFuture<'a, Result<wire::ModelResult, Error>> {
        Box::pin(self.infer(request))
    }
    fn progress<'a>(&'a self, progress: &'a wire::Progress) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let mut metrics = self.metrics.lock().unwrap();
            if let Some(stage) = &progress.stage
                && stage != &metrics.stage
            {
                let elapsed = metrics.since.elapsed().as_millis() as u64;
                let prior = metrics.stage.clone();
                *metrics.phases_ms.entry(prior).or_default() += elapsed;
                metrics.stage = stage.clone();
                metrics.since = Instant::now();
                eprintln!("phase: {stage}");
            }
            if let Some(review) = &progress.review {
                metrics
                    .reviews
                    .insert(review.execution_id.clone(), review.clone());
            }
            Ok(())
        })
    }
    fn finish<'a>(&'a self, _: &'a wire::Result) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
}

fn arg<T: std::str::FromStr>(key: &str, default: T) -> T {
    let args: Vec<_> = std::env::args().collect();
    args.windows(2)
        .find(|p| p[0] == key)
        .map(|p| p[1].parse().unwrap_or_else(|_| panic!("invalid {key}")))
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sessions = arg("--sessions", 9usize);
    let tokens = arg("--tokens", 50000usize);
    let sizes = arg("--sizes", String::new());
    let targets = if sizes.is_empty() {
        vec![tokens; sessions]
    } else {
        sizes
            .split(',')
            .map(str::parse)
            .collect::<Result<Vec<usize>, _>>()?
    };
    let concurrency = arg("--concurrency", 2usize);
    let budget = arg("--budget", 1.0f64);
    let call_limit = arg("--calls", 100usize);
    let seconds = arg("--seconds", 900u64);
    let io_ms = arg("--io-ms", 0u64);
    let model = arg("--model", "claude-haiku-5-5".to_string());
    let mode = arg("--mode", "replay".to_string());
    let tape = PathBuf::from(arg("--tape", "investigation-tape".to_string()));
    let output = PathBuf::from(arg("--output", "investigation-result.json".to_string()));
    assert!((1..=27).contains(&sessions) && (1000..=500_000).contains(&tokens));
    assert!(targets.len() == sessions && targets.iter().all(|v| (1000..=500_000).contains(v)));
    assert!(
        (1..=16).contains(&concurrency) && budget.is_finite() && budget > 0.0 && budget <= 10.0
    );
    assert!(call_limit > 0 && call_limit <= 500 && seconds > 0 && seconds <= 3600);
    assert!(["record", "replay", "live", "evidence"].contains(&mode.as_str()));
    tokio::fs::create_dir_all(&tape).await?;
    let fixture = Fixture::new(&targets);
    let claim = fixture.claim(&model, concurrency);
    let models = if mode == "replay" || mode == "evidence" {
        None
    } else {
        let provider = match std::env::var("BENCH_PROVIDER").as_deref() {
            Ok("openai_compatible") => Provider::OpenAiCompatible,
            Ok("openai") => Provider::OpenAi,
            Ok("anthropic") => Provider::Anthropic,
            Err(_) => return Err("Set BENCH_PROVIDER explicitly before a paid run".into()),
            _ => panic!("BENCH_PROVIDER must be anthropic, openai or openai_compatible"),
        };
        let key = std::env::var("BENCH_API_KEY").or_else(|_| std::env::var("ANTHROPIC_API_KEY"))?;
        Some(AnalysisModels::new(
            lens_analysis::bundled_catalog()?,
            vec![Deployment {
                name: model.clone(),
                model: model.clone(),
                provider,
                api_base: std::env::var("BENCH_API_BASE")
                    .ok()
                    .map(|url| url.parse())
                    .transpose()?,
                api_key: Secret::new(key),
                input_cost_per_token: std::env::var("BENCH_INPUT_RATE")
                    .ok()
                    .map(|v| v.parse())
                    .transpose()?,
                output_cost_per_token: std::env::var("BENCH_OUTPUT_RATE")
                    .ok()
                    .map(|v| v.parse())
                    .transpose()?,
                capacity: lens_inference::ModelCapacity::default(),
                output_limits: lens_inference::OutputLimits {
                    max_completion_tokens: Some(4096.try_into()?),
                    ..Default::default()
                },
            }],
            TransportLimits {
                timeout: Duration::from_secs(120),
                max_response_bytes: 2 * 1024 * 1024,
            },
        )?)
    };
    let backend = Arc::new(Backend {
        fixture,
        models,
        model: model.clone(),
        mode: mode.clone(),
        tape,
        budget,
        call_limit,
        io_delay: Duration::from_millis(io_ms),
        metrics: Mutex::new(Metrics {
            stage: "setup".into(),
            since: Instant::now(),
            phases_ms: BTreeMap::new(),
            calls: Vec::new(),
            failures: Vec::new(),
            pages: 0,
            cost: 0.0,
            reserved: 0.0,
            attempts: 0,
            reviews: BTreeMap::new(),
        }),
    });
    let client = JobClient::local(
        backend.clone(),
        concurrency,
        Arc::new(tokio::sync::Semaphore::new(concurrency)),
    );
    if mode == "evidence" {
        let report = load::run(&backend, &claim, client, concurrency).await?;
        std::fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
        println!("{}", report);
        return Ok(());
    }
    let workspace = litellm_lens::evidence::Workspace::new(
        backend.fixture.sample.executions.clone(),
        client.clone(),
    );
    let preflight = litellm_lens::sandbox::execute(&workspace, &serde_json::from_value(json!({
        "action":"python", "code":"print('lens benchmark ready')", "execution_ids":[backend.fixture.sample.executions[0].id], "span_ids":[backend.fixture.parts[0][0].span_id],
    }))?).await?;
    if preflight["exit_code"] != 0 || preflight["output_complete"] != true {
        eprintln!("Python evidence tool preflight failed: {preflight}");
        return Err(
            "Configure the production Python evidence runtime before running model benchmarks"
                .into(),
        );
    }
    let started = Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(seconds),
        pipeline::analyze(&claim, backend.fixture.sample.clone(), client),
    )
    .await;
    let mut metrics = backend.metrics.lock().unwrap();
    let last = metrics.stage.clone();
    let elapsed = metrics.since.elapsed().as_millis() as u64;
    *metrics.phases_ms.entry(last).or_default() += elapsed;
    let (result, error) = match outcome {
        Ok(Ok(result)) => (Some(result), None),
        Ok(Err(error)) => (None, Some(error.to_string())),
        Err(_) => (None, Some("wall time limit exceeded".into())),
    };
    let valid = error.is_none()
        && result.as_ref().is_some_and(|r| r.error.is_empty())
        && metrics.reviews.len() == sessions
        && metrics
            .reviews
            .values()
            .all(|r| !r.cannot_assess && r.extraction.as_ref().is_some_and(|e| !e.cannot_assess));
    let binary_sha256 = format!(
        "{:x}",
        Sha256::digest(std::fs::read(std::env::current_exe()?)?)
    );
    let settings_sha256 = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&claim.job.settings)?)
    );
    let report = json!({"mode":mode,"model":model,"binary_sha256":binary_sha256,"settings_sha256":settings_sha256,"python_tool_preflight":preflight,"fixture_sha256":backend.fixture.sha256,"sessions":sessions,"target_tokens_per_session":targets,"fixture_tokens_cl100k":backend.fixture.tokens,"concurrency":concurrency,"io_delay_ms":io_ms,"budget_usd":budget,"call_limit":call_limit,"wall_limit_seconds":seconds,"wall_ms":started.elapsed().as_millis(),"phases_ms":metrics.phases_ms,"model_calls":metrics.calls,"model_failures":metrics.failures,"attempts":metrics.attempts,"content_pages":metrics.pages,"paid_catalog_cost_usd":metrics.cost,"cpu_and_memory":"capture with /usr/bin/time -v; excludes remote inference hardware","expected":backend.fixture.expected(),"valid_quality_sample":valid,"quality":result.as_ref().filter(|_|valid).map(|r| backend.fixture.score(r)),"result":result,"error":error,"reviews":metrics.reviews.values().collect::<Vec<_>>()});
    std::fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"output":output,"wall_ms":report["wall_ms"],"phases_ms":report["phases_ms"],"calls":report["attempts"],"pages":report["content_pages"],"paid_catalog_cost_usd":report["paid_catalog_cost_usd"],"quality":report["quality"],"error":report["error"],"pipeline_error":report["result"]["error"]})
    );
    if !valid {
        return Err("Incomplete investigation, excluded from benchmark quality samples".into());
    }
    Ok(())
}
