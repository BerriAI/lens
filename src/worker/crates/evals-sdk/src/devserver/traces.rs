use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use crate::{engine::failure, model::CaseResult};

pub(super) struct Trace {
    pub version: String,
    pub ended: bool,
    pub updated: Instant,
}

pub(super) struct Submission {
    pub result: CaseResult,
    pub received: Instant,
}

pub(super) struct Wait {
    pub idle: Duration,
    pub timeout: Duration,
}

impl Default for Wait {
    fn default() -> Self {
        Self {
            idle: Duration::from_secs(120),
            timeout: Duration::from_secs(1200),
        }
    }
}

impl Wait {
    pub fn resolve(
        &self,
        submission: &Submission,
        traces: &BTreeMap<(String, String), Trace>,
        version: &str,
        now: Instant,
    ) -> Option<CaseResult> {
        let Some(reference) = &submission.result.trace else {
            return Some(submission.result.clone());
        };
        let elapsed = Duration::from_millis(submission.result.duration_ms.unwrap_or(0));
        let deadline = submission.received + self.timeout.saturating_sub(elapsed);
        if let Some(trace) = traces.get(&(reference.attribute.clone(), reference.value.clone())) {
            if trace.version != version {
                return Some(failure(
                    "VersionMismatch",
                    "Trace agent.version differs from the eval run version",
                ));
            }
            let closed = if trace.ended {
                trace.updated
            } else {
                trace.updated + self.idle
            };
            if closed <= now && closed <= deadline {
                return Some(submission.result.clone());
            }
        } else if reference.value.starts_with("pass") || reference.value.starts_with("fail") {
            return Some(submission.result.clone());
        }
        (now >= deadline).then(|| {
            failure(
                "TimeoutError",
                "Trace did not close before the trial timeout",
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TraceRef;
    use rstest::rstest;

    #[rstest]
    #[case::still_running(false, 119, 1200, "sha", None)]
    #[case::root_ended(true, 0, 1200, "sha", Some("ok"))]
    #[case::idle_closed(false, 120, 1200, "sha", Some("ok"))]
    #[case::timeout_before_idle(false, 60, 60, "sha", Some("TimeoutError"))]
    #[case::stale_build(true, 0, 1200, "stale", Some("VersionMismatch"))]
    fn trace_closure_and_build_identity(
        #[case] ended: bool,
        #[case] elapsed: u64,
        #[case] timeout: u64,
        #[case] version: &str,
        #[case] expected: Option<&str>,
    ) {
        let start = Instant::now();
        let submission = Submission {
            received: start,
            result: CaseResult {
                trace: Some(TraceRef {
                    attribute: "session.id".into(),
                    value: "accepted".into(),
                }),
                ..CaseResult::default()
            },
        };
        let traces = BTreeMap::from([(
            ("session.id".into(), "accepted".into()),
            Trace {
                version: version.into(),
                ended,
                updated: start,
            },
        )]);
        let wait = Wait {
            timeout: Duration::from_secs(timeout),
            ..Wait::default()
        };
        let result = wait.resolve(
            &submission,
            &traces,
            "sha",
            start + Duration::from_secs(elapsed),
        );
        assert_eq!(
            result.as_ref().map(|value| value
                .error
                .as_ref()
                .map_or("ok", |error| error.r#type.as_str())),
            expected
        );
    }

    #[rstest]
    #[case::unseen(0, false, "TimeoutError")]
    #[case::late_root(0, true, "TimeoutError")]
    #[case::request_used_budget(900, true, "TimeoutError")]
    fn missing_or_late_traces_respect_remaining_budget(
        #[case] duration_ms: u64,
        #[case] observed: bool,
        #[case] expected: &str,
    ) {
        let start = Instant::now();
        let submission = Submission {
            received: start,
            result: CaseResult {
                trace: Some(TraceRef {
                    attribute: "session.id".into(),
                    value: "accepted".into(),
                }),
                duration_ms: Some(duration_ms),
                ..CaseResult::default()
            },
        };
        let ended = start + Duration::from_millis(if duration_ms == 0 { 1100 } else { 200 });
        let traces = if observed {
            BTreeMap::from([(
                ("session.id".into(), "accepted".into()),
                Trace {
                    version: "sha".into(),
                    ended: true,
                    updated: ended,
                },
            )])
        } else {
            BTreeMap::new()
        };
        let wait = Wait {
            timeout: Duration::from_secs(1),
            ..Wait::default()
        };
        let result = wait
            .resolve(&submission, &traces, "sha", start + Duration::from_secs(2))
            .unwrap();
        assert_eq!(result.error.unwrap().r#type, expected);
    }
}
