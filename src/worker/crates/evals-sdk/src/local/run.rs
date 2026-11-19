use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use futures_util::{StreamExt, stream};

use super::{AgentResult, Attempt, Outcome, Progress, RunMeta, Summary, Target, TargetStats, Task};
use crate::{Error, Result};

const RECOMMEND_WINDOW: f64 = 0.05;

pub fn load_tasks(path: &Path) -> Result<Vec<Task>> {
    let mut reader = csv::Reader::from_path(path)?;
    let headers = reader.headers()?.clone();
    let column = |name: &'static str| headers.iter().position(|header| header.trim() == name);
    let id = column("id").ok_or(Error::Invalid {
        message: "Tasks CSV is missing a required column",
        value: "id".to_owned(),
    })?;
    let prompt = column("prompt").ok_or(Error::Invalid {
        message: "Tasks CSV is missing a required column",
        value: "prompt".to_owned(),
    })?;
    let expected = column("expected");
    let tasks: Vec<Task> = reader
        .records()
        .map(|record| {
            let record = record?;
            let field = |index: usize| record.get(index).unwrap_or_default().to_owned();
            Ok(Task {
                id: field(id),
                prompt: field(prompt),
                expected: expected.map(field).filter(|value| !value.is_empty()),
                metadata: headers
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        ![Some(id), Some(prompt), expected].contains(&Some(*index))
                    })
                    .map(|(index, name)| (name.trim().to_owned(), field(index)))
                    .collect(),
            })
        })
        .collect::<Result<_>>()?;
    let mut ids = HashSet::new();
    if let Some(task) = tasks
        .iter()
        .find(|task| task.id.trim().is_empty() || !ids.insert(&task.id))
    {
        return Err(Error::Invalid {
            message: "Task ids must be nonempty and unique",
            value: task.id.clone(),
        });
    }
    Ok(tasks)
}

pub fn new_run_id() -> String {
    format!("eval_{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

fn percentile(sorted: &[f64], rank: f64) -> f64 {
    match sorted.len() {
        0 => 0.0,
        count => {
            let index = ((rank * count as f64).ceil() as usize).clamp(1, count) - 1;
            sorted[index]
        }
    }
}

fn repeats(attempt: &Attempt) -> usize {
    let calls = &attempt.result.tool_calls;
    calls
        .iter()
        .enumerate()
        .filter(|(index, call)| {
            calls[..*index]
                .iter()
                .any(|earlier| earlier.name == call.name && earlier.arguments == call.arguments)
        })
        .count()
}

fn stats(target: &Target, attempts: &[&Attempt]) -> TargetStats {
    let count = |predicate: &dyn Fn(&Attempt) -> bool| {
        attempts.iter().filter(|attempt| predicate(attempt)).count() as u32
    };
    let misused = |attempt: &Attempt| attempt.result.tool_calls.iter().any(|call| call.error);
    let failed = |attempt: &Attempt| matches!(attempt.result.outcome, Outcome::Fail { .. });
    let calls: usize = attempts
        .iter()
        .map(|attempt| attempt.result.tool_calls.len())
        .sum();
    let repeated: usize = attempts.iter().map(|attempt| repeats(attempt)).sum();
    let mut latencies: Vec<f64> = attempts
        .iter()
        .map(|attempt| attempt.latency_seconds)
        .collect();
    latencies.sort_by(f64::total_cmp);
    TargetStats {
        target: target.clone(),
        pass: count(&|attempt| matches!(attempt.result.outcome, Outcome::Pass)),
        fail: count(&failed),
        wrong_answer: count(&|attempt| failed(attempt) && !misused(attempt)),
        tool_misuse: count(&|attempt| failed(attempt) && misused(attempt)),
        error: count(&|attempt| matches!(attempt.result.outcome, Outcome::Error { .. })),
        model_cost: attempts.iter().map(|attempt| attempt.result.cost).sum(),
        judge_cost: attempts
            .iter()
            .map(|attempt| attempt.result.judge_cost)
            .sum(),
        tool_calls_per_case: match attempts.len() {
            0 => 0.0,
            cases => calls as f64 / cases as f64,
        },
        repeat_rate: match calls {
            0 => 0.0,
            calls => repeated as f64 / calls as f64,
        },
        p50_seconds: percentile(&latencies, 0.50),
        p95_seconds: percentile(&latencies, 0.95),
    }
}

fn recommend(stats: &[TargetStats]) -> Option<usize> {
    let best = stats.iter().map(TargetStats::pass_rate).fold(0.0, f64::max);
    stats
        .iter()
        .enumerate()
        .filter(|(_, stat)| stat.pass > 0 && stat.pass_rate() >= best - RECOMMEND_WINDOW)
        .min_by(|(_, left), (_, right)| {
            (left.cost() / f64::from(left.pass)).total_cmp(&(right.cost() / f64::from(right.pass)))
        })
        .map(|(index, _)| index)
}

pub fn summarize(meta: &RunMeta, attempts: &[Attempt]) -> Summary {
    let stats: Vec<TargetStats> = meta
        .targets
        .iter()
        .map(|target| {
            let mine: Vec<&Attempt> = attempts
                .iter()
                .filter(|attempt| &attempt.target == target)
                .collect();
            stats(target, &mine)
        })
        .collect();
    Summary {
        run_id: meta.id.clone(),
        task_count: meta.task_count,
        recommended: recommend(&stats),
        stats,
    }
}

fn runs(root: &Path) -> PathBuf {
    root.join(".lens").join("runs")
}

fn attempts_path(directory: &Path) -> PathBuf {
    directory.join("attempts.jsonl")
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).map_err(Error::Checkpoint)?,
    )?;
    Ok(())
}

fn read_attempts(path: &Path) -> Result<Vec<Attempt>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    BufReader::new(File::open(path)?)
        .lines()
        .filter(|line| line.as_ref().map_or(true, |line| !line.trim().is_empty()))
        .map(|line| serde_json::from_str(&line?).map_err(Error::Checkpoint))
        .collect()
}

pub fn load_run(root: &Path, run_id: &str) -> Result<(RunMeta, Vec<Attempt>)> {
    let directory = runs(root).join(run_id);
    let meta = serde_json::from_slice(&fs::read(directory.join("meta.json"))?)
        .map_err(Error::Checkpoint)?;
    Ok((meta, read_attempts(&attempts_path(&directory))?))
}

pub fn latest_run(root: &Path) -> Option<String> {
    fs::read_dir(runs(root))
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let modified = entry.metadata().ok()?.modified().ok()?;
            entry
                .path()
                .join("meta.json")
                .exists()
                .then_some((modified, entry.file_name()))
        })
        .max_by_key(|(modified, _)| *modified)
        .and_then(|(_, name)| name.into_string().ok())
}

pub async fn execute<F, Fut>(
    root: &Path,
    meta: RunMeta,
    tasks: Vec<Task>,
    invoke: F,
    progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<Summary>
where
    F: Fn(Task, Target) -> Fut + Send + Sync,
    Fut: Future<Output = AgentResult> + Send,
{
    let directory = runs(root).join(&meta.id);
    fs::create_dir_all(&directory)?;
    write_json(&directory.join("meta.json"), &meta)?;
    let journal = attempts_path(&directory);
    let previous = read_attempts(&journal)?;
    let done: HashSet<(String, Target)> = previous
        .iter()
        .map(|attempt| (attempt.task_id.clone(), attempt.target.clone()))
        .collect();
    progress(Progress::Started {
        meta: meta.clone(),
        resumed: previous.len(),
    });
    let pending: Vec<(Task, Target)> = tasks
        .iter()
        .flat_map(|task| {
            meta.targets
                .iter()
                .map(move |target| (task.clone(), target.clone()))
        })
        .filter(|(task, target)| !done.contains(&(task.id.clone(), target.clone())))
        .collect();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&journal)?;
    let invoke = &invoke;
    let mut completed = stream::iter(pending)
        .map(|(task, target)| async move {
            let started = Instant::now();
            let task_id = task.id.clone();
            let result = invoke(task, target.clone()).await;
            Attempt {
                task_id,
                target,
                latency_seconds: started.elapsed().as_secs_f64(),
                result,
            }
        })
        .buffer_unordered(meta.concurrency.max(1));
    let mut attempts = previous;
    while let Some(attempt) = completed.next().await {
        let line = serde_json::to_string(&attempt).map_err(Error::Checkpoint)?;
        writeln!(file, "{line}")?;
        file.flush()?;
        progress(Progress::Finished(Box::new(attempt.clone())));
        attempts.push(attempt);
    }
    let summary = summarize(&meta, &attempts);
    write_json(&directory.join("summary.json"), &summary)?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::percentile;
    use rstest::rstest;

    #[rstest]
    #[case::p50(0.50, 5.0)]
    #[case::p95(0.95, 10.0)]
    #[case::p10(0.10, 1.0)]
    fn percentile_uses_nearest_rank(#[case] rank: f64, #[case] expected: f64) {
        let sorted = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert_eq!(percentile(&sorted, rank), expected);
    }
}
