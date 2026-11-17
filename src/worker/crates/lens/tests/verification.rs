use litellm_lens::{
    Error,
    activity::Tracker,
    agent::{self, Assignment, Output},
    evidence::Workspace,
    wire,
};
use rstest::rstest;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

mod replay;

macro_rules! rule {
    () => {
        "----------------------------------------------------------------------"
    };
}

const TOOL: &str = concat!(
    "Input: {\"command\":\"python3 -m unittest\"}\nOutput: Exit code 1\nFAIL: test_refund (tests.RefundTest.test_refund)\n",
    rule!(),
    "\nAssertionError: refund_status=failed\n\nRan 1 test in 0.001s\nStatus: STATUS_CODE_ERROR Shell command failed"
);
const REPLY: &str = r#"Input:
Output: {"role":"assistant","content":"Your refund is complete.\nThe \"refund\" tool confirmed it."}
Status: STATUS_CODE_UNSET "#;
const FAILURE: &str = "AssertionError: refund_status=failed";

#[derive(Clone)]
enum Reply {
    Turn(Value),
    Raw(&'static str),
    Truncated(&'static str),
}

struct Session {
    server: MockServer,
    calls: Arc<AtomicUsize>,
}

impl Session {
    async fn start(replies: Vec<Reply>) -> Self {
        let server = MockServer::start().await;
        let sample: wire::Sample =
            serde_json::from_str(include_str!("fixtures/sample.json")).unwrap();
        let execution = sample.executions[0].clone();
        Mock::given(method("GET"))
            .and(path("/replay/lens-test/job-test/content"))
            .respond_with(move |request: &Request| {
                let id = request
                    .url
                    .query_pairs()
                    .find(|(key, _)| key == "execution_id")
                    .unwrap()
                    .1
                    .into_owned();
                let parts = if id == "run-test" {
                    json!([
                        {"execution_id": id, "span_id": "span-reply", "name": "assistant", "kind": "llm", "content": REPLY, "truncated": false},
                        {"execution_id": id, "span_id": "span-tool", "name": "Bash", "kind": "tool", "content": TOOL, "truncated": false}
                    ])
                } else {
                    json!([{"execution_id": id, "span_id": "span-other", "name": "Bash", "kind": "tool", "content": "Input: ls\nOutput: README.md\nStatus: STATUS_CODE_UNSET ", "truncated": false}])
                };
                ResponseTemplate::new(200).set_body_json(json!({
                    "execution": wire::Execution { id, ..execution.clone() },
                    "parts": parts,
                }))
            })
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/replay/lens-test/job-test/progress"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        Mock::given(method("POST"))
            .and(path("/replay/lens-test/job-test/model"))
            .respond_with(move |_: &Request| {
                let reply = replies
                    .get(counted.fetch_add(1, Ordering::SeqCst))
                    .cloned()
                    .expect("the model was called more often than scripted");
                ResponseTemplate::new(200).set_body_json(match reply {
                    Reply::Turn(turn) => json!({"content": turn.to_string(), "cost": 0}),
                    Reply::Raw(content) => json!({"content": content, "cost": 0}),
                    Reply::Truncated(content) => {
                        json!({"content": content, "cost": 0, "finish_reason": "length"})
                    }
                })
            })
            .mount(&server)
            .await;
        Self { server, calls }
    }

    async fn run<T: Output>(&self) -> Result<T, Error> {
        let claim: wire::Claim = serde_json::from_str(include_str!("fixtures/claim.json")).unwrap();
        let sample: wire::Sample =
            serde_json::from_str(include_str!("fixtures/sample.json")).unwrap();
        let other = wire::Execution {
            id: "other-run-0001".into(),
            ..sample.executions[0].clone()
        };
        let client = replay::client(&self.server.uri(), "lens-test", "job-test", 2);
        let workspace = Workspace::new(vec![sample.executions[0].clone(), other], client.clone());
        let tracker = Tracker::start(
            &client,
            "verification".into(),
            wire::ActivityPhase::Review,
            "Verify".into(),
            vec![],
        )
        .await
        .unwrap();
        agent::run(
            &claim,
            &workspace,
            Assignment {
                stage: "test",
                task: "Review the run".into(),
                purpose: wire::ModelRequestPurpose::Extract,
                supplied: json!({}),
            },
            &tracker,
        )
        .await
    }

    async fn model_requests(&self) -> Vec<wire::ModelRequest> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.url.path().ends_with("/model"))
            .map(|request| request.body_json().unwrap())
            .collect()
    }
}

fn observation(execution: &str, span: &str, quote: &str) -> Value {
    json!({"check_id": "refund", "summary": "The refund failed", "evidence": [
        {"execution_id": execution, "span_id": span, "quote": quote, "role": "support"}
    ]})
}

fn extraction(observations: Vec<Value>) -> Reply {
    Reply::Turn(json!({"result": {"observations": observations}}))
}

#[rstest]
#[case::exact("run-test", "span-tool", FAILURE, "span-tool", vec![FAILURE])]
#[case::collapsed_whitespace(
    "run-test", "span-tool",
    "AssertionError: refund_status=failed Ran 1 test in 0.001s",
    "span-tool", vec!["AssertionError: refund_status=failed\n\nRan 1 test in 0.001s"]
)]
#[case::unescaped_recording(
    "run-test", "span-reply",
    "Your refund is complete.\nThe \"refund\" tool confirmed it.",
    "span-reply", vec![r#"Your refund is complete.\nThe \"refund\" tool confirmed it."#]
)]
#[case::shortened_with_ellipsis(
    "run-test", "span-tool",
    "FAIL: test_refund (tests.RefundTest.test_refund) ... AssertionError: refund_status=failed",
    "span-tool", vec![concat!("FAIL: test_refund (tests.RefundTest.test_refund)\n", rule!(), "\nAssertionError: refund_status=failed")]
)]
#[case::cited_the_wrong_span("run-test", "span-reply", FAILURE, "span-tool", vec![FAILURE])]
#[case::unknown_span("run-test", "span-missing", FAILURE, "span-tool", vec![FAILURE])]
#[case::mistyped_execution("run-tesT", "span-tool", FAILURE, "span-tool", vec![FAILURE])]
#[tokio::test]
async fn recorded_evidence_is_accepted_as_the_recorded_text_without_another_model_call(
    #[case] execution: &str,
    #[case] span: &str,
    #[case] quote: &str,
    #[case] recorded_span: &str,
    #[case] recorded: Vec<&str>,
) {
    let session = Session::start(vec![extraction(vec![observation(execution, span, quote)])]).await;
    let output: wire::Extraction = session.run().await.unwrap();
    let evidence = &output.observations[0].evidence;
    assert_eq!(
        evidence
            .iter()
            .map(|cited| (
                cited.execution_id.as_str(),
                cited.span_id.as_str(),
                cited.quote.as_str()
            ))
            .collect::<Vec<_>>(),
        recorded
            .iter()
            .map(|quote| ("run-test", recorded_span, *quote))
            .collect::<Vec<_>>()
    );
    assert_eq!(session.calls.load(Ordering::SeqCst), 1);
}

#[rstest]
#[tokio::test]
async fn rejected_citation_is_returned_with_the_recorded_text_it_resembles() {
    let paraphrase = "AssertionError: the refund_status was failed";
    let session = Session::start(vec![
        extraction(vec![observation("run-test", "span-tool", paraphrase)]),
        extraction(vec![observation("run-test", "span-tool", FAILURE)]),
    ])
    .await;
    let output: wire::Extraction = session.run().await.unwrap();
    assert_eq!(output.observations[0].evidence[0].quote.as_str(), FAILURE);
    let requests = session.model_requests().await;
    assert_eq!(requests.len(), 2);
    let repair: Value =
        serde_json::from_str(&requests[1].messages.last().unwrap().content).unwrap();
    let rejected = &repair["rejected"][0];
    assert_eq!(rejected["path"], "result.observations[0].evidence[0]");
    assert_eq!(
        rejected["rejected_citation"],
        json!({"execution_id": "run-test", "span_id": "span-tool", "quote": paraphrase})
    );
    assert!(
        rejected["recorded_text_near_quote"]
            .as_str()
            .unwrap()
            .contains(FAILURE)
    );
}

#[rstest]
#[case::invented_quote(observation(
    "run-test",
    "span-tool",
    "the database migration was rolled back"
))]
#[case::unknown_execution(observation("run-zzzz", "span-tool", FAILURE))]
#[case::no_supporting_quote(json!({"check_id": "refund", "summary": "Unsupported", "evidence": []}))]
#[case::disabled_check(json!({"check_id": "latency", "summary": "Slow", "evidence": [
    {"execution_id": "run-test", "span_id": "span-tool", "quote": FAILURE, "role": "support"}
]}))]
#[tokio::test]
async fn unverifiable_observations_are_dropped_after_repairs_and_verified_ones_kept(
    #[case] unverifiable: Value,
) {
    let answer = extraction(vec![
        unverifiable,
        observation("run-test", "span-tool", FAILURE),
    ]);
    let session = Session::start(vec![answer.clone(), answer.clone(), answer]).await;
    let output: wire::Extraction = session.run().await.unwrap();
    assert_eq!(output.observations.len(), 1);
    assert_eq!(output.observations[0].evidence[0].quote.as_str(), FAILURE);
    assert_eq!(session.calls.load(Ordering::SeqCst), 3);
}

#[rstest]
#[tokio::test]
async fn result_with_nothing_verifiable_fails_and_reports_the_real_attempt_count() {
    let answer = extraction(vec![observation(
        "run-test",
        "span-tool",
        "the database migration was rolled back",
    )]);
    let session = Session::start(vec![answer.clone(), answer.clone(), answer]).await;
    let error = session.run::<wire::Extraction>().await.err().unwrap();
    assert!(
        error.to_string().starts_with(
            "PythonAgentTurn[Extraction] response invalid after 3 attempts: 1 of the cited results could not be verified"
        ),
        "{error}"
    );
    assert_eq!(session.calls.load(Ordering::SeqCst), 3);
}

#[rstest]
#[tokio::test]
async fn finding_bookkeeping_mistakes_are_corrected_without_another_model_call() {
    let session = Session::start(vec![Reply::Turn(json!({"result": {"findings": [{
        "title": "Refund success was falsely reported",
        "description": "The agent said the refund completed even though its tool returned a failure",
        "check_id": "refund",
        "check_ids": ["refund", "not-enabled"],
        "kind": "issue",
        "existing_finding_id": "not-a-saved-finding",
        "merged_finding_ids": ["not-a-saved-finding"],
        "evidence": [{"execution_id": "run-test", "span_id": "span-tool", "quote": FAILURE, "role": "support"}],
        "brief": {"problem": "A failed refund was reported as successful", "user_goal": "Receive a refund", "what_happened": "The refund tool failed", "test_cases": [{"input": "A failed refund", "expected": "The agent explains the failure"}]}
    }]}}))])
    .await;
    let output: wire::Findings = session.run().await.unwrap();
    let finding = &output.findings[0];
    assert_eq!(finding.check_ids, vec!["refund"]);
    assert_eq!(finding.existing_finding_id, None);
    assert!(finding.merged_finding_ids.is_empty());
    assert_eq!(session.calls.load(Ordering::SeqCst), 1);
}

#[rstest]
#[case::fenced(vec![Reply::Raw("```json\n{\"result\":{\"observations\":[]}}\n```")], 1)]
#[case::trailing_commentary(vec![Reply::Raw("{\"result\":{\"observations\":[]}}\nNothing else to report.")], 1)]
#[case::corrected_on_the_last_attempt(vec![
    Reply::Turn(json!({"result": {"observations": {}}})),
    Reply::Turn(json!({"result": {"result_note": "fine", "observations": []}})),
    Reply::Turn(json!({"result": {"observations": []}})),
], 3)]
#[tokio::test]
async fn decorated_or_repairable_responses_complete_the_review(
    #[case] replies: Vec<Reply>,
    #[case] calls: usize,
) {
    let session = Session::start(replies).await;
    let output: wire::Extraction = session.run().await.unwrap();
    assert!(output.observations.is_empty());
    assert_eq!(session.calls.load(Ordering::SeqCst), calls);
    let requests = session.model_requests().await;
    let repairs = requests
        .last()
        .unwrap()
        .messages
        .iter()
        .filter(|message| message.content.contains("validation_errors"))
        .count();
    assert_eq!(repairs, calls - 1);
}

#[rstest]
#[tokio::test]
async fn response_cut_off_at_the_output_limit_is_reported_as_truncation() {
    let cut = Reply::Truncated("{\"result\":{\"observations\":[{\"check_id\":\"refu");
    let session = Session::start(vec![cut.clone(), cut.clone(), cut]).await;
    let error = session.run::<wire::Extraction>().await.err().unwrap();
    assert!(
        error.to_string().starts_with(
            "PythonAgentTurn[Extraction] response invalid after 3 attempts: The response reached the model output limit"
        ),
        "{error}"
    );
    assert_eq!(session.calls.load(Ordering::SeqCst), 3);
}

#[rstest]
#[tokio::test]
async fn run_cannot_be_declared_unassessable_before_its_spans_were_read() {
    let unread = Reply::Turn(
        json!({"result": {"cannot_assess": true, "reasoning": "No span contents were supplied."}}),
    );
    let session = Session::start(vec![
        unread.clone(),
        Reply::Turn(json!({"tools": [{"action": "read", "execution_id": "run-test"}]})),
        unread,
    ])
    .await;
    let output: wire::Extraction = session.run().await.unwrap();
    assert!(output.cannot_assess);
    let requests = session.model_requests().await;
    assert_eq!(requests.len(), 3);
    let repair: Value =
        serde_json::from_str(&requests[1].messages.last().unwrap().content).unwrap();
    assert_eq!(repair["rejected"][0]["path"], "result");
    assert!(
        repair["rejected"][0]["problem"]
            .as_str()
            .unwrap()
            .starts_with("You have not read the recorded spans")
    );
}

#[rstest]
#[tokio::test]
async fn listing_the_spans_does_not_count_as_reading_the_run() {
    let unread = Reply::Turn(json!({"result": {"cannot_assess": true}}));
    let session = Session::start(vec![
        unread.clone(),
        Reply::Turn(json!({"tools": [{"action": "catalog", "execution_id": "run-test"}]})),
        unread.clone(),
        unread,
    ])
    .await;
    let error = session.run::<wire::Extraction>().await.err().unwrap();
    assert!(
        error
            .to_string()
            .contains("You have not read the recorded spans"),
        "{error}"
    );
}

#[rstest]
#[tokio::test]
async fn run_that_is_never_read_fails_instead_of_being_skipped() {
    let unread = Reply::Turn(json!({"result": {"cannot_assess": true}}));
    let session = Session::start(vec![unread.clone(), unread.clone(), unread]).await;
    let error = session.run::<wire::Extraction>().await.err().unwrap();
    assert!(
        error
            .to_string()
            .contains("You have not read the recorded spans"),
        "{error}"
    );
}
