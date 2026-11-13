mod support;
use lens_contract::{
    investigations::Scope,
    signals::{SignalAttemptStatus, SignalConfig, SignalEvidence},
    worker::Execution,
};
use lens_signals::{DecisionRequest, Decisions, DecisionsError, Question, classify, signal_state};
use rstest::rstest;
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};
use support::*;

#[rstest]
#[tokio::test]
async fn classifier_sends_recorded_steps_questions_and_logging_tags(
    reader: Reader,
    completion: Completion,
    config: SignalConfig,
    scope: Scope,
    execution: Execution,
) {
    let result = classify(&reader, &completion, &scope, &execution, &config).await;
    assert_eq!(result.status, SignalAttemptStatus::Classified);
    assert_eq!(result.model, config.model);
    assert_eq!(
        result.scores,
        BTreeMap::from([("a".into(), 0.9), ("b".into(), 0.2)])
    );
    assert!(result.error.is_empty());
    let calls = completion.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let request = &calls[0];
    assert_eq!(request.model, config.model);
    assert_eq!(request.timeout, Duration::from_secs(60));
    assert_eq!(request.tags, vec!["litellm-lens-signals"]);
    assert_eq!(
        request.state.task,
        "An AI agent run recorded as a trace. Judge only what the user and the agent said and did in these steps."
    );
    assert_eq!(
        serde_json::to_value(&request.state.steps).unwrap(),
        json!([{"kind":"user","name":"question","content":"[L000] user text"}])
    );
    assert_eq!(
        serde_json::to_value(
            request
                .questions
                .iter()
                .filter(|(id, _)| !id.starts_with("__evidence_"))
                .collect::<BTreeMap<_, _>>()
        )
        .unwrap(),
        json!({"a":{"type":"noul","instructions":"First question?"},"b":{"type":"noul","instructions":"Second question?"}})
    );
    let Question::Choice {
        instructions,
        criteria,
    } = &request.questions["__evidence_0"]
    else {
        panic!("Expected an evidence choice");
    };
    assert!(instructions.contains("First question?"));
    assert_eq!(
        criteria.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["L000", "none"]
    );
    assert_eq!(request.questions.len(), 4);
}

#[rstest]
#[case::small(1)]
#[case::last_direct_choice(251)]
#[case::first_grouped_choice(252)]
#[case::beyond_previous_limits(400)]
#[case::reported_large(9050)]
#[case::reported_extreme(120940)]
#[tokio::test]
async fn evidence_choice_resolves_to_the_recorded_span_and_literal_quote(
    reader: Reader,
    completion: Completion,
    config: SignalConfig,
    scope: Scope,
    execution: Execution,
    #[case] preceding_steps: usize,
) {
    let quote = "You ignored my request again. This still does not work.";
    let earlier = "Earlier request. ".repeat(8);
    let choice = format!("L{:03}", preceding_steps + 2);
    let reader = Reader {
        contents: BTreeMap::from([
            (
                "".into(),
                page(
                    (0..preceding_steps).map(|_| part(&earlier)).collect(),
                    Some("second"),
                ),
            ),
            (
                "second".into(),
                page(vec![part("More context")], Some("third")),
            ),
            (
                "third".into(),
                page(vec![part("Latest request")], Some("fourth")),
            ),
            (
                "fourth".into(),
                page(
                    vec![lens_contract::worker::TracePart {
                        span_id: "problem-step".into(),
                        ..part(quote)
                    }],
                    None,
                ),
            ),
        ]),
        ..reader
    };
    let completion = EvidenceCompletion {
        target: preceding_steps + 2,
        calls: completion.calls,
    };
    let result = classify(&reader, &completion, &scope, &execution, &config).await;
    assert_eq!(result.status, SignalAttemptStatus::Classified);
    assert_eq!(
        result.evidence,
        BTreeMap::from([(
            "a".into(),
            SignalEvidence {
                span_id: "problem-step".into(),
                quote: quote.into(),
            }
        )])
    );
    let calls = completion.calls.lock().unwrap();
    let mut expected_calls = 1;
    let mut remaining = preceding_steps + 3;
    while remaining > 254 {
        let width = remaining.div_ceil(254);
        remaining = (remaining - 1) % width + 1;
        if remaining > 1 {
            expected_calls += 1;
        }
    }
    assert_eq!(calls.len(), expected_calls);
    assert_eq!(calls[0].state.steps.len(), preceding_steps + 3);
    assert_eq!(
        calls[0].state.steps[0].content,
        format!("[L000] {}", earlier.trim())
    );
    assert_eq!(
        calls[0].state.steps.last().unwrap().content,
        format!("[{choice}] {quote}")
    );
    let Question::Choice { criteria, .. } = &calls[0].questions["__evidence_0"] else {
        panic!("Expected an evidence choice");
    };
    assert!(criteria.len() <= 255);
    let invalid = calls.iter().flat_map(|call| call.questions.values()).any(
        |question| matches!(question, Question::Choice { criteria, .. } if criteria.len() > 255),
    );
    assert!(!invalid);
    assert!(
        matches!(&calls.last().unwrap().questions["__evidence_0"], Question::Choice { criteria, .. } if criteria.contains_key(&choice))
    );
    assert_eq!(
        *reader.content_calls.lock().unwrap(),
        vec!["", "second", "third", "fourth"]
    );
}

#[rstest]
#[case::missing(json!(null))]
#[case::unknown_candidate(json!({"type":"choice","choice":"invented","confidence":0.8}))]
#[case::none(json!({"type":"choice","choice":"none","confidence":0.8}))]
#[case::refused(json!({"type":"refusal"}))]
#[case::wrong_type(json!({"type":"noul","choice":"L000","confidence":0.8}))]
#[case::bad_confidence(json!({"type":"choice","choice":"L000","confidence":1.2}))]
#[case::missing_confidence(json!({"type":"choice","choice":"L000"}))]
#[tokio::test]
async fn unavailable_evidence_preserves_valid_signal_scores(
    reader: Reader,
    completion: Completion,
    config: SignalConfig,
    scope: Scope,
    execution: Execution,
    #[case] evidence: Value,
) {
    let completion = Completion {
        response: json!({"answers":{"a":{"type":"noul","noul":0.9},
            "b":{"type":"noul","noul":0.2},"__evidence_0":evidence}}),
        ..completion
    };
    let result = classify(&reader, &completion, &scope, &execution, &config).await;
    assert_eq!(result.status, SignalAttemptStatus::Classified);
    assert_eq!(
        result.scores,
        BTreeMap::from([("a".into(), 0.9), ("b".into(), 0.2)])
    );
    assert!(result.evidence.is_empty());
    assert_eq!(completion.calls.lock().unwrap().len(), 1);
}

#[rstest]
#[case::missing(json!({}))]
#[case::wrong_type(json!({"b":{"type":"score","noul":0.4}}))]
#[case::out_of_range(json!({"b":{"type":"noul","noul":1.1}}))]
#[case::null(json!({"b":null}))]
#[tokio::test]
async fn invalid_answers_preserve_partial_valid_scores(
    reader: Reader,
    completion: Completion,
    config: SignalConfig,
    scope: Scope,
    execution: Execution,
    #[case] mut answers: Value,
) {
    answers["a"] = json!({"type":"noul","noul":0.8});
    let result = classify(
        &reader,
        &Completion {
            response: json!({"answers":answers}),
            ..completion
        },
        &scope,
        &execution,
        &config,
    )
    .await;
    assert_eq!(result.status, SignalAttemptStatus::Failed);
    assert!(result.failure.is_none());
    assert_eq!(result.scores, BTreeMap::from([("a".into(), 0.8)]));
    assert_eq!(
        result.error,
        "Decisions response omitted a configured noul answer"
    );
}

#[rstest]
#[case::source(true, false)]
#[case::provider(false, true)]
#[tokio::test]
async fn failures_do_not_persist_internal_secrets(
    reader: Reader,
    completion: Completion,
    config: SignalConfig,
    scope: Scope,
    execution: Execution,
    #[case] source: bool,
    #[case] provider: bool,
) {
    let result = classify(
        &Reader {
            fail_content: source,
            ..reader
        },
        &Completion {
            fail: provider,
            ..completion
        },
        &scope,
        &execution,
        &config,
    )
    .await;
    assert_eq!(result.status, SignalAttemptStatus::Failed);
    assert_eq!(result.model, config.model);
    assert!(result.scores.is_empty());
    assert!(!result.error.is_empty());
    assert!(!result.error.contains("secret"));
}

#[rstest]
#[case::not_object(json!(null))]
#[case::positional_object(json!([{"a":{"type":"noul","noul":0.8},"b":{"type":"noul","noul":0.2}}]))]
#[case::missing_answers(json!({}))]
#[case::invalid_answers(json!({"answers":[]}))]
#[tokio::test]
async fn invalid_responses_fail_classification(
    reader: Reader,
    completion: Completion,
    config: SignalConfig,
    scope: Scope,
    execution: Execution,
    #[case] response: Value,
) {
    let result = classify(
        &reader,
        &Completion {
            response,
            ..completion
        },
        &scope,
        &execution,
        &config,
    )
    .await;
    assert_eq!(result.status, SignalAttemptStatus::Failed);
    assert_eq!(result.error, "Decisions response is invalid");
}

#[rstest]
#[case::short(2000)]
#[case::long(70_000)]
#[tokio::test]
async fn parts_preserve_all_unicode_content(
    reader: Reader,
    scope: Scope,
    execution: Execution,
    #[case] count: usize,
) {
    let content = format!("{}{}", "雪".repeat(800), "界".repeat(count - 800));
    let reader = Reader {
        contents: BTreeMap::from([("".into(), page(vec![part(&content)], None))]),
        ..reader
    };
    let state = signal_state(&reader, &scope, &execution).await.unwrap();
    assert_eq!(state.steps[0].content, content);
}

#[rstest]
#[tokio::test]
async fn content_follows_every_page(reader: Reader, scope: Scope, execution: Execution) {
    let reader = Reader {
        contents: BTreeMap::from([
            ("".into(), page(vec![part("first")], Some("second"))),
            ("second".into(), page(vec![part("second")], Some("third"))),
            ("third".into(), page(vec![part("third")], Some("fourth"))),
            ("fourth".into(), page(vec![part("fourth")], None)),
        ]),
        ..reader
    };
    let state = signal_state(&reader, &scope, &execution).await.unwrap();
    assert_eq!(
        state
            .steps
            .iter()
            .map(|step| step.content.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second", "third", "fourth"]
    );
    assert_eq!(
        *reader.content_calls.lock().unwrap(),
        vec!["", "second", "third", "fourth"]
    );
}

#[rstest]
#[case::short(40)]
#[case::long(50)]
#[tokio::test]
async fn transcript_preserves_all_steps(
    reader: Reader,
    scope: Scope,
    execution: Execution,
    #[case] count: usize,
) {
    let parts = (0..count)
        .map(|i| part(format!("{i:03}{}", "雪".repeat(997))))
        .collect::<Vec<_>>();
    let reader = Reader {
        contents: BTreeMap::from([("".into(), page(parts, None))]),
        ..reader
    };
    let state = signal_state(&reader, &scope, &execution).await.unwrap();
    assert_eq!(state.steps.len(), count);
    assert_eq!(state.steps[0].content, format!("000{}", "雪".repeat(997)));
    assert_eq!(
        state.steps.last().unwrap().content,
        format!("{:03}{}", count - 1, "雪".repeat(997))
    );
    assert_eq!(state.steps[15].content, format!("015{}", "雪".repeat(997)));
}

struct EvidenceCompletion {
    target: usize,
    calls: std::sync::Mutex<Vec<DecisionRequest>>,
}

impl Decisions for EvidenceCompletion {
    async fn complete(&self, request: &DecisionRequest) -> Result<Value, DecisionsError> {
        self.calls.lock().unwrap().push(request.clone());
        let answers = request
            .questions
            .iter()
            .map(|(id, question)| {
                let answer = match question {
                    Question::Noul { .. } => json!({"type":"noul","noul":0.9}),
                    Question::Choice { criteria, .. } => {
                        assert!(criteria.len() <= 255);
                        let choice = if id == "__evidence_0" {
                            criteria
                                .keys()
                                .find(|key| {
                                    let mut bounds = key.split("..").filter_map(|part| {
                                        part.strip_prefix('L')?.parse::<usize>().ok()
                                    });
                                    let Some(start) = bounds.next() else {
                                        return false;
                                    };
                                    let end = bounds.next().unwrap_or(start);
                                    start <= self.target && self.target <= end
                                })
                                .map(String::as_str)
                                .unwrap_or("none")
                        } else {
                            "none"
                        };
                        json!({"type":"choice","choice":choice,"confidence":0.9})
                    }
                };
                (id.clone(), answer)
            })
            .collect::<serde_json::Map<_, _>>();
        Ok(json!({"answers":answers}))
    }
}

struct Refusal(u16);
impl Decisions for Refusal {
    async fn complete(&self, _: &DecisionRequest) -> Result<Value, DecisionsError> {
        Err(DecisionsError::Provider {
            status: self.0,
            retry_after_seconds: Some(120),
        })
    }
}

#[rstest]
#[case::invalid(400, true)]
#[case::unauthorized(401, false)]
#[case::forbidden(403, false)]
#[case::unknown(404, false)]
#[case::large(413, true)]
#[case::unprocessable(422, true)]
#[case::limited(429, false)]
#[case::server(500, false)]
#[case::unavailable(503, false)]
#[tokio::test]
async fn refusal_preserves_retry_policy(
    reader: Reader,
    config: SignalConfig,
    scope: Scope,
    execution: Execution,
    #[case] status: u16,
    #[case] permanent: bool,
) {
    use lens_contract::signals::SignalFailure;
    let attempt = classify(&reader, &Refusal(status), &scope, &execution, &config).await;
    assert_eq!(
        attempt.failure,
        Some(if permanent {
            SignalFailure::Permanent
        } else {
            SignalFailure::Transient {
                retry_after_seconds: Some(120),
            }
        })
    );
}

#[rstest]
#[case::none("none", 0.9)]
#[case::unoffered_leaf("L000", 0.9)]
#[case::invalid_confidence("L000..L001", 1.1)]
#[tokio::test]
async fn unavailable_group_evidence_does_not_fan_out_or_discard_scores(
    reader: Reader,
    completion: Completion,
    config: SignalConfig,
    scope: Scope,
    execution: Execution,
    #[case] choice: &str,
    #[case] confidence: f64,
) {
    let reader = Reader {
        contents: BTreeMap::from([(
            "".into(),
            page((0..400).map(|_| part("Recorded request")).collect(), None),
        )]),
        ..reader
    };
    let completion = Completion {
        response: json!({"answers":{"a":{"type":"noul","noul":0.9},"b":{"type":"noul","noul":0.2},"__evidence_0":{"type":"choice","choice":choice,"confidence":confidence}}}),
        ..completion
    };
    let result = classify(&reader, &completion, &scope, &execution, &config).await;
    assert_eq!(result.status, SignalAttemptStatus::Classified);
    assert!(result.evidence.is_empty());
    assert_eq!(result.scores.len(), config.signals.len());
    assert_eq!(completion.calls.lock().unwrap().len(), 1);
}

struct FollowupFailure(u16);
impl Decisions for FollowupFailure {
    async fn complete(&self, request: &DecisionRequest) -> Result<Value, DecisionsError> {
        if request.questions.contains_key("a") {
            return Ok(json!({"answers":{
                "a":{"type":"noul","noul":0.9}, "b":{"type":"noul","noul":0.2},
                "__evidence_0":{"type":"choice","choice":"L000..L001","confidence":0.9},
                "__evidence_1":{"type":"choice","choice":"L400","confidence":0.9}
            }}));
        }
        match self.0 {
            0 => Err(DecisionsError::Timeout),
            1 => Ok(json!([])),
            status => Err(DecisionsError::Provider {
                status,
                retry_after_seconds: Some(120),
            }),
        }
    }
}
#[rstest]
#[case::timeout(0, true)]
#[case::malformed(1, false)]
#[case::invalid(400, false)]
#[case::large(413, false)]
#[case::unprocessable(422, false)]
#[case::rate_limited(429, true)]
#[case::unavailable(503, true)]
#[tokio::test]
async fn evidence_followup_failure_keeps_completed_scores_and_evidence(
    reader: Reader,
    config: SignalConfig,
    scope: Scope,
    execution: Execution,
    #[case] failure: u16,
    #[case] cooldown: bool,
) {
    let reader = Reader {
        contents: BTreeMap::from([(
            "".into(),
            page((0..401).map(|_| part("Recorded request")).collect(), None),
        )]),
        ..reader
    };
    let result = classify(
        &reader,
        &FollowupFailure(failure),
        &scope,
        &execution,
        &config,
    )
    .await;
    assert_eq!(result.status, SignalAttemptStatus::Classified);
    assert_eq!(
        result.scores,
        BTreeMap::from([("a".into(), 0.9), ("b".into(), 0.2)])
    );
    assert_eq!(result.evidence.len(), 1);
    assert_eq!(result.evidence["b"].quote, "Recorded request");
    assert!(result.error.is_empty());
    assert_eq!(result.failure.is_some(), cooldown);
    let identity = lens_contract::feedback::TraceIdentity {
        trace_id: execution.trace_id.clone(),
        trace_ref: execution.trace_ref.clone(),
    };
    let stored = lens_contract::signals::StoredTraceSignal {
        trace_id: identity.trace_id.clone(),
        trace_ref: identity.trace_ref.clone(),
        config_key: lens_signals::config_key(&config),
        span_count: execution.span_count,
        claimed_until: None,
        classified_at: Some(now()),
        data: serde_json::to_value(&result).unwrap(),
    };
    let public = lens_signals::trace_signals(&identity, Some(&stored), &config).unwrap();
    assert_eq!(
        public.status,
        lens_contract::signals::TraceSignalStatus::Classified
    );
    assert_eq!(public.flags.len(), 1);
    assert_eq!(public.flags[0].score, 0.9);
}

struct ContextLimited {
    calls: std::sync::Mutex<Vec<DecisionRequest>>,
    state_limit: usize,
}

impl Decisions for ContextLimited {
    async fn complete(&self, request: &DecisionRequest) -> Result<Value, DecisionsError> {
        self.calls.lock().unwrap().push(request.clone());
        if request
            .questions
            .values()
            .any(|q| matches!(q, Question::Choice { .. }))
            || serde_json::to_vec(&request.state).unwrap().len() > self.state_limit
        {
            return Err(DecisionsError::Context);
        }
        Ok(json!({"answers":{"a":{"type":"noul","noul":0.9},"b":{"type":"noul","noul":0.2}}}))
    }
}

#[rstest]
#[case::questions_only("small trace".into(), false)]
#[case::large_unicode("Начало 🦀 ".repeat(4000), true)]
#[tokio::test]
async fn context_refusal_drops_evidence_then_reduces_state_without_losing_scores(
    reader: Reader,
    config: SignalConfig,
    scope: Scope,
    execution: Execution,
    #[case] content: String,
    #[case] partial: bool,
) {
    let reader = Reader {
        contents: BTreeMap::from([("".into(), page(vec![part(content)], None))]),
        ..reader
    };
    let completion = ContextLimited {
        calls: Default::default(),
        state_limit: 2048,
    };
    let result = classify(&reader, &completion, &scope, &execution, &config).await;
    assert_eq!(result.status, SignalAttemptStatus::Classified);
    assert_eq!(result.scores["a"], 0.9);
    assert_eq!(result.partial, partial);
    assert!(result.evidence.is_empty());
    assert!(result.failure.is_none());
    let calls = completion.calls.lock().unwrap();
    assert_eq!(calls[0].state, calls[1].state);
    assert!(
        calls[1]
            .questions
            .values()
            .all(|q| matches!(q, Question::Noul { .. }))
    );
    assert!(calls.len() < 16);
    assert!(
        calls
            .windows(2)
            .all(|pair| pair[1].timeout <= pair[0].timeout)
    );
    if partial {
        assert!(calls.last().unwrap().state.task.starts_with("Partial"));
        assert!(
            calls.last().unwrap().state.steps[0]
                .content
                .contains("Начало")
        );
    }
}

#[rstest]
#[tokio::test]
async fn irreducible_context_refusals_stop_and_remain_retryable(
    reader: Reader,
    config: SignalConfig,
    scope: Scope,
    execution: Execution,
) {
    let completion = ContextLimited {
        calls: Default::default(),
        state_limit: 1,
    };
    let result = classify(&reader, &completion, &scope, &execution, &config).await;
    assert_eq!(result.status, SignalAttemptStatus::Failed);
    assert!(matches!(
        result.failure,
        Some(lens_contract::signals::SignalFailure::Transient { .. })
    ));
    assert_eq!(completion.calls.lock().unwrap().len(), 2);
}
