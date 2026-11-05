use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use tokio::process::Command;

use crate::{Error, State};

const MEMORY_LIMIT_KIB: u64 = 512 * 1024;
const RESTART_DELAY: Duration = Duration::from_secs(300);
const START_HEADROOM: u64 = 320 * 1024 * 1024;
const RUN_HEADROOM: u64 = 64 * 1024 * 1024;

fn remaining_memory(limit: &str, used: &str) -> Option<u64> {
    Some(
        limit
            .trim()
            .parse::<u64>()
            .ok()?
            .saturating_sub(used.trim().parse().ok()?),
    )
}

async fn container_headroom() -> Option<u64> {
    for (limit, usage) in [
        ("/sys/fs/cgroup/memory.max", "/sys/fs/cgroup/memory.current"),
        (
            "/sys/fs/cgroup/memory/memory.limit_in_bytes",
            "/sys/fs/cgroup/memory/memory.usage_in_bytes",
        ),
    ] {
        if let (Ok(limit), Ok(usage)) = (
            tokio::fs::read_to_string(limit).await,
            tokio::fs::read_to_string(usage).await,
        ) && let Some(remaining) = remaining_memory(&limit, &usage)
        {
            return Some(remaining);
        }
    }
    None
}

pub fn enabled() -> Result<bool, Error> {
    match std::env::var("LENS_TASK_PROFILES_ENABLED").as_deref() {
        Ok("false" | "0") => Ok(false),
        Err(std::env::VarError::NotPresent) | Ok("true" | "1") => Ok(true),
        _ => Err(Error::Configuration(
            "LENS_TASK_PROFILES_ENABLED must be true or false",
        )),
    }
}

fn cache_directory() -> PathBuf {
    if let Some(path) = std::env::var_os("LENS_PROFILE_MODEL_CACHE") {
        return PathBuf::from(path);
    }
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    root.join("lens/task-profiles")
}

fn duty_cycle_delay(elapsed: Duration) -> Duration {
    elapsed.saturating_mul(4).max(Duration::from_millis(250))
}

pub async fn work() -> Result<(), Error> {
    let config = crate::config::Config::from_env()?;
    let client = crate::config::http_client()?;
    let mut classifier = lens_profiles::Classifier::new(cache_directory());
    loop {
        let started = std::time::Instant::now();
        let result =
            litellm_traces_clickhouse::profiles::project(&client, &config.storage, &mut classifier)
                .await;
        let delay = match result {
            Ok(true) => duty_cycle_delay(started.elapsed()),
            Ok(false) => Duration::from_secs(30),
            Err(_) => {
                tracing::warn!("Task grouping deferred; storage or local model is unavailable");
                RESTART_DELAY
            }
        };
        tokio::time::sleep(delay).await;
    }
}

async fn resident_kib(pid: u32) -> Option<u64> {
    if cfg!(target_os = "linux") {
        let status = tokio::fs::read_to_string(format!("/proc/{pid}/status"))
            .await
            .ok()?;
        return status.lines().find_map(|line| {
            line.strip_prefix("VmRSS:")?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        });
    }
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !result.status.success() || result.stdout.len() > 64 {
        return None;
    }
    std::str::from_utf8(&result.stdout)
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub async fn supervise(state: Arc<State>) {
    loop {
        if !state
            .schema_ready
            .load(std::sync::atomic::Ordering::Acquire)
        {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }
        if container_headroom()
            .await
            .is_some_and(|available| available < START_HEADROOM)
        {
            tracing::warn!("Task grouping deferred because container memory headroom is low");
            tokio::time::sleep(RESTART_DELAY).await;
            continue;
        }
        if let Ok(executable) = std::env::current_exe() {
            let child = Command::new(executable)
                .arg("task-profile-worker")
                .env("TOKENIZERS_PARALLELISM", "false")
                .env("OMP_NUM_THREADS", "1")
                .env("OPENBLAS_NUM_THREADS", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn();
            if let Ok(mut child) = child
                && let Some(pid) = child.id()
            {
                loop {
                    tokio::select! {
                        _ = child.wait() => break,
                        _ = tokio::time::sleep(Duration::from_secs(1)) => {
                            if resident_kib(pid).await.is_none_or(|memory| memory > MEMORY_LIMIT_KIB)
                                || container_headroom().await.is_some_and(|available| available < RUN_HEADROOM) {
                                tracing::warn!("Task grouping worker stopped by its memory guard");
                                let _ = child.kill().await;
                                break;
                            }
                        }
                    }
                }
            }
        }
        tokio::time::sleep(RESTART_DELAY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::short(10, 250)]
    #[case::slow(3000, 12000)]
    fn classification_has_a_bounded_duty_cycle(#[case] work_ms: u64, #[case] sleep_ms: u64) {
        assert_eq!(
            duty_cycle_delay(Duration::from_millis(work_ms)),
            Duration::from_millis(sleep_ms)
        );
    }

    #[rstest]
    #[tokio::test]
    async fn memory_guard_reads_the_current_process() {
        assert!(
            resident_kib(std::process::id())
                .await
                .is_some_and(|rss| rss > 0)
        );
    }

    #[rstest]
    #[case::available("1000", "250", Some(750))]
    #[case::over_limit("1000", "1200", Some(0))]
    #[case::unlimited("max", "1200", None)]
    #[case::invalid("1000", "invalid", None)]
    fn container_limits_preserve_headroom(
        #[case] limit: &str,
        #[case] used: &str,
        #[case] remaining: Option<u64>,
    ) {
        assert_eq!(remaining_memory(limit, used), remaining);
    }
}
