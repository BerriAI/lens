mod support;
use lens_contract::{
    investigations::Scope,
    signals::{SignalAttemptStatus, SignalConfig, SignalEvidence},
    worker::Execution,
};
use lens_signals::{Question, classify, signal_state};
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
#[case::beyond_previous_limits(400)]
#[tokio::test]
async fn evidence_choice_resolves_to_the_recorded_span_and_literal_quote_in_one_call(
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
    let completion = Completion {
        response: json!({"answers":{
            "a":{"type":"noul","noul":0.9},"b":{"type":"noul","noul":0.2},
            "__evidence_0":{"type":"choice","choice":choice,"confidence":0.9},
            "__evidence_1":{"type":"choice","choice":"none","confidence":0.8}
        }}),
        ..completion
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
    assert_eq!(calls.len(), 1);
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
    assert_eq!(criteria.len(), preceding_steps + 4);
    assert!(criteria.contains_key(&choice));
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
