mod support;
use lens_contract::{
    investigations::Scope,
    signals::{SignalAttemptStatus, SignalConfig},
    worker::Execution,
};
use lens_signals::{classify, signal_state};
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
        json!([{"kind":"user","name":"question","content":"user text"}])
    );
    assert_eq!(
        serde_json::to_value(&request.questions).unwrap(),
        json!({"a":{"type":"noul","instructions":"First question?"},"b":{"type":"noul","instructions":"Second question?"}})
    );
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
#[case::at_limit(2000, false)]
#[case::over_limit(2001, true)]
#[tokio::test]
async fn part_excerpt_preserves_unicode_head_and_tail(
    reader: Reader,
    scope: Scope,
    execution: Execution,
    #[case] count: usize,
    #[case] bounded: bool,
) {
    let content = format!("{}{}", "雪".repeat(800), "界".repeat(count - 800));
    let reader = Reader {
        contents: BTreeMap::from([("".into(), page(vec![part(&content)], None))]),
        ..reader
    };
    let state = signal_state(&reader, &scope, &execution).await.unwrap();
    let expected = if bounded {
        format!(
            "{}\n[... 1 characters omitted ...]\n{}",
            "雪".repeat(800),
            "界".repeat(1200)
        )
    } else {
        content
    };
    assert_eq!(state.steps[0].content, expected);
}

#[rstest]
#[tokio::test]
async fn content_stops_at_three_pages_even_with_more_cursors(
    reader: Reader,
    scope: Scope,
    execution: Execution,
) {
    let reader = Reader {
        contents: BTreeMap::from([
            ("".into(), page(vec![part("first")], Some("second"))),
            ("second".into(), page(vec![part("second")], Some("third"))),
            ("third".into(), page(vec![part("third")], Some("ignored"))),
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
        vec!["first", "second", "third"]
    );
    assert_eq!(
        *reader.content_calls.lock().unwrap(),
        vec!["", "second", "third"]
    );
}

#[rstest]
#[case::at_limit(40, false)]
#[case::over_limit(50, true)]
#[tokio::test]
async fn transcript_preserves_first_and_last_steps_with_omission_count(
    reader: Reader,
    scope: Scope,
    execution: Execution,
    #[case] count: usize,
    #[case] bounded: bool,
) {
    let parts = (0..count)
        .map(|i| part(format!("{i:03}{}", "雪".repeat(997))))
        .collect::<Vec<_>>();
    let reader = Reader {
        contents: BTreeMap::from([("".into(), page(parts, None))]),
        ..reader
    };
    let state = signal_state(&reader, &scope, &execution).await.unwrap();
    assert_eq!(state.steps.len(), if bounded { 41 } else { 40 });
    assert_eq!(state.steps[0].content, format!("000{}", "雪".repeat(997)));
    assert_eq!(
        state.steps.last().unwrap().content,
        format!("{:03}{}", count - 1, "雪".repeat(997))
    );
    if bounded {
        assert_eq!(state.steps[15].kind, "omitted");
        assert_eq!(state.steps[15].name, "");
        assert_eq!(state.steps[15].content, "10 steps omitted");
        assert!(state.steps[16].content.starts_with("025"));
    }
}
