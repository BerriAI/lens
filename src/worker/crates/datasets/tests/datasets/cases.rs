use super::support::{case, message};
use lens_contract::datasets::{CaseSource, DatasetCase, DatasetRole, DatasetToolCall, SkipReason};
use lens_datasets::{
    Candidate, Limits, RevisionProblem, Scope, can_access, case_chars, case_from_span, case_id,
    export_jsonl, included_cases, make_case, revision_cases, revision_problem,
};
use litellm_traces::{ChatRole, SpanDetail, UiContent, UiMessage};
use rstest::rstest;
use serde_json::Value;

#[rstest]
#[case::plain(include_str!("../../../parity/fixtures/datasets/05_build_from_plain_text.json"))]
#[case::jsonl(include_str!("../../../parity/fixtures/datasets/06_build_from_json_lines.json"))]
#[case::revision(include_str!("../../../parity/fixtures/datasets/07_save_revision_with_included_and_excluded_cases.json"))]
fn hashes_match_recorded_python_cases(#[case] fixture: &str) {
    let fixture: Value = serde_json::from_str(fixture).unwrap();
    let cases: Vec<DatasetCase> =
        serde_json::from_value(fixture["response"]["body"]["cases"].clone()).unwrap();
    for case in cases {
        assert_eq!(
            case_id(&case.messages, &case.reply, &case.tool_calls),
            case.id
        );
    }
}

#[rstest]
fn unicode_and_special_characters_hash_the_exact_materialized_json() {
    use sha2::{Digest, Sha256};
    let expected = r#"{"messages":[{"role":"user","content":"a\n\"é😀\u0000","name":"","tool_calls":[]}],"reply":"résultat","tool_calls":[]}"#;
    assert_eq!(
        case_id(&[message("a\n\"é😀\0")], "résultat", &[]),
        format!("{:x}", Sha256::digest(expected.as_bytes()))
    );
}

#[rstest]
fn revisions_rehash_content_and_keep_the_first_duplicate_with_its_metadata() {
    let first = DatasetCase {
        id: "supplied".into(),
        expected: "first".into(),
        included: false,
        source: CaseSource {
            trace_id: "t".into(),
            ..Default::default()
        },
        agent_version: "v7".into(),
        ..case("question")
    };
    let duplicate = DatasetCase {
        id: "different".into(),
        expected: "second".into(),
        included: true,
        ..first.clone()
    };
    let distinct = case("other");
    let revised = revision_cases(&[first.clone(), duplicate, distinct.clone()]);
    assert_eq!(
        revised,
        vec![
            DatasetCase {
                id: case_id(&first.messages, &first.reply, &first.tool_calls),
                ..first
            },
            DatasetCase {
                id: case_id(&distinct.messages, &distinct.reply, &distinct.tool_calls),
                ..distinct
            }
        ]
    );
}

#[rstest]
fn content_hash_includes_message_roles_names_and_tool_history() {
    let original = case("q");
    let history = DatasetCase {
        messages: vec![lens_contract::datasets::DatasetMessage {
            role: DatasetRole::Assistant,
            name: "actor".into(),
            tool_calls: vec![DatasetToolCall {
                name: "lookup".into(),
                arguments: "{}".into(),
            }],
            ..message("q")
        }],
        ..original.clone()
    };
    assert_ne!(
        case_id(&original.messages, "", &[]),
        case_id(&history.messages, "", &[])
    );
}

#[rstest]
fn case_size_counts_unicode_scalars_in_every_content_field_only() {
    let mut item = case("😀");
    item.messages[0].name = "é".into();
    item.messages[0].tool_calls = vec![DatasetToolCall {
        name: "α".into(),
        arguments: "β".into(),
    }];
    item.reply = "λ".into();
    item.tool_calls = vec![DatasetToolCall {
        name: "γ".into(),
        arguments: "δ".into(),
    }];
    item.expected = "ζ".into();
    item.id = "unmeasured".repeat(100);
    item.agent_version = "unmeasured".repeat(100);
    item.source.trace_id = "unmeasured".repeat(100);
    assert_eq!(case_chars(&item), 8);
    assert_eq!(
        revision_problem(
            &[item.clone()],
            Limits {
                max_cases: 1,
                max_case_chars: 8
            }
        ),
        None
    );
    assert_eq!(
        revision_problem(
            &[item],
            Limits {
                max_cases: 1,
                max_case_chars: 7
            }
        ),
        Some(RevisionProblem::CaseTooLarge(7))
    );
}

#[rstest]
#[case::at_count(1, 1, None)]
#[case::over_count(2, 1, Some(RevisionProblem::TooManyCases(1)))]
#[case::empty_zero(0, 0, None)]
#[case::negative_limit(0, -1, Some(RevisionProblem::TooManyCases(-1)))]
fn revision_count_limits_include_excluded_cases(
    #[case] count: usize,
    #[case] max_cases: i64,
    #[case] expected: Option<RevisionProblem>,
) {
    let cases = vec![
        DatasetCase {
            included: false,
            ..case("q")
        };
        count
    ];
    assert_eq!(
        revision_problem(
            &cases,
            Limits {
                max_cases,
                ..Default::default()
            }
        ),
        expected
    );
}

#[rstest]
#[case::count(
    RevisionProblem::TooManyCases(200),
    "A dataset holds at most 200 cases"
)]
#[case::chars(
    RevisionProblem::CaseTooLarge(20_000),
    "Each case must be at most 20000 characters"
)]
fn revision_errors_preserve_the_public_message(
    #[case] error: RevisionProblem,
    #[case] expected: &str,
) {
    assert_eq!(error.to_string(), expected);
}

#[rstest]
#[case::blank_reply(vec![], " \t\r\n\u{1c}\u{1f}", vec![], 20, Some(SkipReason::NoContent))]
#[case::reply_only(vec![], "answer", vec![], 20, None)]
#[case::blank_message(vec![message("")], "", vec![], 20, None)]
#[case::tool_only(vec![], "", vec![DatasetToolCall {name:"call".into(),arguments:"{}".into()}], 20, None)]
#[case::at_size(vec![message("😀😀")], "", vec![], 2, None)]
#[case::over_size(vec![message("😀😀")], "", vec![], 1, Some(SkipReason::TooLarge))]
#[case::negative_size(vec![message("")], "", vec![], -1, Some(SkipReason::TooLarge))]
fn make_case_preserves_content_admission(
    #[case] messages: Vec<lens_contract::datasets::DatasetMessage>,
    #[case] reply: &str,
    #[case] tools: Vec<DatasetToolCall>,
    #[case] max_case_chars: i64,
    #[case] reason: Option<SkipReason>,
) {
    let source = CaseSource {
        span_id: "s".into(),
        ..Default::default()
    };
    let candidate = make_case(
        messages.clone(),
        reply.into(),
        tools.clone(),
        source.clone(),
        String::new(),
        "v7".into(),
        Limits {
            max_case_chars,
            ..Default::default()
        },
    );
    match (candidate, reason) {
        (Candidate::Skipped(skipped), Some(reason)) => {
            assert_eq!(skipped.reason, reason);
            assert_eq!(skipped.source, source);
        }
        (Candidate::Case(case), None) => {
            assert_eq!(case.messages, messages);
            assert_eq!(case.reply, reply);
            assert_eq!(case.tool_calls, tools);
            assert_eq!(case.source, source);
            assert_eq!(case.agent_version, "v7");
            assert!(case.included);
        }
        other => panic!("unexpected admission: {other:?}"),
    }
}

fn ui(role: ChatRole, content: &str) -> UiMessage {
    UiMessage {
        role,
        content: content.into(),
        name: None,
        tool_calls: None,
    }
}

#[rstest]
fn span_conversion_keeps_history_and_only_assistant_output() {
    let detail = SpanDetail {
        span_id: "actual".into(),
        input_ui: UiContent::Messages {
            messages: vec![
                ui(ChatRole::System, "rules"),
                UiMessage {
                    name: Some("lookup".into()),
                    ..ui(ChatRole::Tool, "result")
                },
                ui(ChatRole::User, "question"),
            ],
        },
        output_ui: UiContent::Messages {
            messages: vec![
                ui(ChatRole::Assistant, "first"),
                ui(ChatRole::Tool, "not an answer"),
                ui(ChatRole::Assistant, ""),
                UiMessage {
                    tool_calls: Some(vec![litellm_traces::UiToolCall {
                        name: "search".into(),
                        arguments: "{\"q\":1}".into(),
                    }]),
                    ..ui(ChatRole::Assistant, "last")
                },
            ],
        },
        input: "raw question".into(),
        output: "raw answer".into(),
        attributes: [("agent.version".into(), "v7".into())].into(),
    };
    let Candidate::Case(case) = case_from_span(
        &detail,
        CaseSource {
            span_id: "stale".into(),
            ..Default::default()
        },
        Limits::default(),
    ) else {
        panic!("case expected")
    };
    assert_eq!(
        case.messages.iter().map(|m| m.role).collect::<Vec<_>>(),
        [DatasetRole::System, DatasetRole::Tool, DatasetRole::User]
    );
    assert_eq!(case.messages[1].name, "lookup");
    assert_eq!(case.reply, "first\n\nlast");
    assert_eq!(
        case.tool_calls,
        [DatasetToolCall {
            name: "search".into(),
            arguments: "{\"q\":1}".into()
        }]
    );
    assert_eq!(
        (case.source.span_id.as_str(), case.agent_version.as_str()),
        ("actual", "v7")
    );
}

#[rstest]
#[case::text(UiContent::Text {text: "display input".into()}, UiContent::Text {text: "display answer".into()}, "raw input", "raw answer", "raw input", "display answer")]
#[case::fields(UiContent::Fields {fields: vec![]}, UiContent::Fields {fields: vec![]}, "{\"q\":1}", "{\"ok\":true}", "{\"q\":1}", "{\"ok\":true}")]
fn non_message_spans_use_the_legacy_fallbacks(
    #[case] input_ui: UiContent,
    #[case] output_ui: UiContent,
    #[case] input: &str,
    #[case] output: &str,
    #[case] expected_input: &str,
    #[case] expected_output: &str,
) {
    let detail = SpanDetail {
        span_id: "s".into(),
        input_ui,
        output_ui,
        input: input.into(),
        output: output.into(),
        attributes: Default::default(),
    };
    let Candidate::Case(case) = case_from_span(&detail, CaseSource::default(), Limits::default())
    else {
        panic!("case expected")
    };
    assert_eq!(case.messages, [message(expected_input)]);
    assert_eq!(case.reply, expected_output);
    assert_eq!(case.agent_version, "");
}

#[rstest]
fn empty_raw_span_is_skipped_with_its_actual_span_id() {
    let detail = SpanDetail {
        span_id: "actual".into(),
        input_ui: UiContent::Fields { fields: vec![] },
        output_ui: UiContent::Text {
            text: " \u{1d}".into(),
        },
        input: " \u{1e}".into(),
        output: "".into(),
        attributes: Default::default(),
    };
    assert_eq!(
        case_from_span(&detail, CaseSource::default(), Limits::default()),
        Candidate::Skipped(lens_contract::datasets::SkippedCase {
            source: CaseSource {
                span_id: "actual".into(),
                ..Default::default()
            },
            reason: SkipReason::NoContent
        })
    );
}

#[rstest]
fn export_contains_only_included_records_with_defaults_and_a_trailing_newline() {
    let kept = case("keep");
    let dropped = DatasetCase {
        included: false,
        ..case("omit")
    };
    assert_eq!(
        included_cases(&[kept.clone(), dropped.clone()]),
        std::slice::from_ref(&kept)
    );
    assert_eq!(
        export_jsonl(&[kept.clone(), dropped]),
        serde_json::to_string(&kept).unwrap() + "\n"
    );
    assert_eq!(export_jsonl(&[]), "");
}

#[rstest]
#[case::admin(Scope {all_teams:true,..Default::default()}, Scope {all_teams:true,..Default::default()}, true)]
#[case::same_team(Scope {team_id:"a".into(),api_key_hash:"one".into(),..Default::default()}, Scope {team_id:"a".into(),api_key_hash:"two".into(),..Default::default()}, true)]
#[case::other_team(Scope {team_id:"a".into(),..Default::default()}, Scope {team_id:"b".into(),..Default::default()}, false)]
#[case::same_key(Scope {api_key_hash:"one".into(),..Default::default()}, Scope {api_key_hash:"one".into(),..Default::default()}, true)]
#[case::other_key(Scope {api_key_hash:"one".into(),..Default::default()}, Scope {api_key_hash:"two".into(),..Default::default()}, false)]
#[case::all_teams_target(Scope::default(), Scope {all_teams:true,..Default::default()}, false)]
fn access_matches_lens_scope_rules(
    #[case] viewer: Scope,
    #[case] target: Scope,
    #[case] expected: bool,
) {
    assert_eq!(can_access(&viewer, &target), expected);
}

#[rstest]
fn default_limits_preserve_large_cases_and_datasets() {
    let mut large = case("large");
    large.reply = "x".repeat(70_000);
    let cases = vec![large; 201];
    assert_eq!(revision_problem(&cases, Limits::default()), None);
}
