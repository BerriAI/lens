use crate::local::{
    AgentResult, Attempt, Outcome, Progress, RunMeta, Summary, Target, TargetStats,
};

pub const RUN_ID: &str = "eval_7f2a";
pub const TASKS: u32 = 3_000;
pub const COMMAND: &str = "/run tasks.csv --config evals.py";
const PICK: usize = 1;
const SHUFFLE: usize = 7_919;

struct Sample {
    harness: &'static str,
    model: &'static str,
    pass: u32,
    fail: u32,
    error: u32,
    model_cost: f64,
    tool_calls: f64,
    repeat_rate: f64,
    p50_seconds: f64,
    p95_seconds: f64,
}

const JUDGE_COST: f64 = 7.50;

const SAMPLES: [Sample; 4] = [
    Sample {
        harness: "claude-agent-sdk",
        model: "sonnet-5.5",
        pass: 2775,
        fail: 150,
        error: 75,
        model_cost: 63.00,
        tool_calls: 3.1,
        repeat_rate: 0.06,
        p50_seconds: 8.4,
        p95_seconds: 19.2,
    },
    Sample {
        harness: "claude-agent-sdk",
        model: "gpt-5-mini",
        pass: 2700,
        fail: 225,
        error: 75,
        model_cost: 28.50,
        tool_calls: 3.4,
        repeat_rate: 0.09,
        p50_seconds: 5.1,
        p95_seconds: 12.8,
    },
    Sample {
        harness: "openai-agents",
        model: "gpt-5-mini",
        pass: 2550,
        fail: 300,
        error: 150,
        model_cost: 31.50,
        tool_calls: 4.2,
        repeat_rate: 0.29,
        p50_seconds: 6.3,
        p95_seconds: 17.5,
    },
    Sample {
        harness: "langgraph",
        model: "gpt-5-mini",
        pass: 2610,
        fail: 270,
        error: 120,
        model_cost: 30.10,
        tool_calls: 3.9,
        repeat_rate: 0.21,
        p50_seconds: 6.0,
        p95_seconds: 15.1,
    },
];

fn target(sample: &Sample) -> Target {
    Target {
        harness: sample.harness.to_owned(),
        model: sample.model.to_owned(),
        label: None,
    }
}

pub fn meta() -> RunMeta {
    RunMeta {
        id: RUN_ID.to_owned(),
        tasks_file: "tasks.csv".to_owned(),
        config_file: "evals.py".to_owned(),
        task_data: None,
        config_hash: None,
        targets: SAMPLES.iter().map(target).collect(),
        task_count: TASKS as usize,
        trials: 1,
        concurrency: 16,
    }
}

pub fn summary() -> Summary {
    Summary {
        run_id: RUN_ID.to_owned(),
        task_count: TASKS as usize,
        trials: 1,
        stats: SAMPLES
            .iter()
            .map(|sample| TargetStats {
                target: target(sample),
                pass: sample.pass,
                fail: sample.fail,
                error: sample.error,
                model_cost: Some(sample.model_cost),
                judge_cost: Some(JUDGE_COST),
                tool_calls_per_case: sample.tool_calls,
                repeat_rate: sample.repeat_rate,
                p50_seconds: sample.p50_seconds,
                p95_seconds: sample.p95_seconds,
                metrics: crate::local::Metrics {
                    metrics_complete: true,
                    ..Default::default()
                },
            })
            .collect(),
        recommended: Some(PICK),
    }
}

fn outcome(sample: &Sample, case: u32) -> Outcome {
    let slot = (case as usize * SHUFFLE) % TASKS as usize;
    match u32::try_from(slot).unwrap_or(0) {
        slot if slot < sample.pass => Outcome::Pass,
        slot if slot < sample.pass + sample.fail => Outcome::Fail {
            reason: "sample failure".to_owned(),
        },
        _ => Outcome::Error {
            kind: "SampleError".to_owned(),
            message: "sample error".to_owned(),
        },
    }
}

pub fn event(index: usize) -> Progress {
    let sample = &SAMPLES[index % SAMPLES.len()];
    let case = (index / SAMPLES.len()) as u32;
    Progress::Finished(Box::new(Attempt {
        task_id: format!("ticket-{:04}", case + 1),
        trial: 1,
        target: target(sample),
        latency_seconds: sample.p50_seconds,
        result: AgentResult {
            outcome: outcome(sample, case),
            output: String::new(),
            cost: Some(sample.model_cost / f64::from(TASKS)),
            judge_cost: Some(JUDGE_COST / f64::from(TASKS)),
            tool_calls: Vec::new(),
            metrics: crate::local::Metrics {
                metrics_complete: true,
                ..Default::default()
            },
            stop_reason: None,
            check_error: None,
            check_detail: None,
            duration_seconds: None,
            run_id: None,
        },
    }))
}

pub fn event_count() -> usize {
    TASKS as usize * SAMPLES.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    fn every_sample_covers_every_task(#[values(0, 1, 2, 3)] index: usize) {
        let sample = &SAMPLES[index];
        assert_eq!(sample.pass + sample.fail + sample.error, TASKS);
    }
}
