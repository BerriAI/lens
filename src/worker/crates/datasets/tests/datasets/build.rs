use super::support::{Detail, Reader, case, request, span, text, trace, trace_source};
use base64::Engine;
use lens_contract::datasets::{
    BuildSource, CaseSource, DatasetCase, FindingSource, SkipReason, SkippedCase,
};
use lens_datasets::{
    Evidence, Finding, Limits, ReadError, Scope, build_cases, case_id, export_jsonl,
};
use litellm_traces::ObservationType;
use rstest::rstest;
use serde_json::{Value, json};

#[rstest]
#[case::plain(include_str!("../../../parity/fixtures/datasets/05_build_from_plain_text.json"))]
#[case::jsonl(include_str!("../../../parity/fixtures/datasets/06_build_from_json_lines.json"))]
#[tokio::test]
async fn builds_match_recorded_python_results(#[case] fixture: &str) {
    let fixture: Value = serde_json::from_str(fixture).unwrap();
    let request = serde_json::from_value(fixture["request"]["body"].clone()).unwrap();
    let result = build_cases(
        &request,
        &Reader::default(),
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        fixture["response"]["body"]
    );
}

#[rstest]
#[case::newline("\n")]
#[case::carriage("\r")]
#[case::windows("\r\n")]
#[case::vertical_tab("\u{b}")]
#[case::form_feed("\u{c}")]
#[case::file_separator("\u{1c}")]
#[case::group_separator("\u{1d}")]
#[case::record_separator("\u{1e}")]
#[case::next_line("\u{85}")]
#[case::line_separator("\u{2028}")]
#[case::paragraph_separator("\u{2029}")]
#[tokio::test]
async fn jsonl_uses_python_line_boundaries(#[case] separator: &str) {
    let text_value = format!(
        "{{\"messages\":[{{\"role\":\"user\",\"content\":\"first\"}}]}}{separator} {separator}{{\"messages\":[{{\"role\":\"user\",\"content\":\"second\"}}]}}{separator}"
    );
    let result = build_cases(
        &request(vec![text(&text_value)]),
        &Reader::default(),
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        result
            .cases
            .iter()
            .map(|case| case.messages[0].content.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert!(result.skipped.is_empty());
}

#[rstest]
#[case::ordinary("Cancel my order\nplease")]
#[case::mixed("{\"messages\":[]}\nplain words")]
#[case::blank(" \n\t ")]
#[case::unicode_prefix("\u{1f}not-json")]
#[tokio::test]
async fn plain_text_is_preserved_as_one_message(#[case] value: &str) {
    let result = build_cases(
        &request(vec![text(value)]),
        &Reader::default(),
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.cases[0].messages, [super::support::message(value)]);
    assert!(result.skipped.is_empty());
}

#[rstest]
#[case::syntax("{not json")]
#[case::missing_messages("{\"reply\":\"no messages\"}")]
#[case::nested_extra("{\"messages\":[{\"role\":\"user\",\"content\":\"q\",\"extra\":true}]}")]
#[case::invalid_role("{\"messages\":[{\"role\":\"function\",\"content\":\"q\"}]}")]
#[case::invalid_type("{\"messages\":[],\"reply\":12}")]
#[case::invalid_source("{\"messages\":[],\"source\":{\"extra\":true}}")]
#[tokio::test]
async fn malformed_jsonl_lines_do_not_discard_valid_lines(#[case] bad: &str) {
    let input = format!(
        "{{\"messages\":[{{\"role\":\"user\",\"content\":\"good\"}}],\"ignored\":true}}\n{bad}"
    );
    let result = build_cases(
        &request(vec![text(&input)]),
        &Reader::default(),
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.cases.len(), 1);
    assert_eq!(result.cases[0].messages[0].content, "good");
    assert_eq!(
        result.skipped,
        [SkippedCase {
            source: CaseSource::default(),
            reason: SkipReason::Invalid
        }]
    );
}

#[rstest]
#[tokio::test]
async fn exported_jsonl_rebuilds_provenance_expected_and_agent_version() {
    let original = DatasetCase {
        expected: "polite refusal".into(),
        agent_version: "v7".into(),
        source: CaseSource {
            trace_id: "t1".into(),
            span_id: "s1".into(),
            ..Default::default()
        },
        ..case("refund?")
    };
    let result = build_cases(
        &request(vec![text(&export_jsonl(std::slice::from_ref(&original)))]),
        &Reader::default(),
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        result.cases,
        [DatasetCase {
            id: case_id(&original.messages, &original.reply, &original.tool_calls),
            ..original
        }]
    );
}

#[rstest]
#[tokio::test]
async fn build_keeps_span_conversation_and_version() {
    let detail = Detail {
        attributes: [("agent.version".into(), "v7".into())].into(),
        ..Detail::chat("refund?")
    };
    let reader = Reader::with_spans(&[("s1", detail)]);
    let result = build_cases(
        &request(vec![trace_source("s1")]),
        &reader,
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.cases[0].messages[1].content, "refund?");
    assert_eq!(result.cases[0].reply, "Done");
    assert_eq!(
        result.cases[0].source,
        CaseSource {
            trace_id: "t1".into(),
            trace_ref: "ref".into(),
            span_id: "s1".into(),
            ..Default::default()
        }
    );
    assert_eq!(result.cases[0].agent_version, "v7");
    assert_eq!(reader.reads.into_inner().unwrap(), ["t1:s1:ref"]);
}

#[rstest]
#[tokio::test]
async fn whole_trace_selects_latest_llm_conversation_and_keeps_tie_order() {
    let reader = Reader {
        trace: Some(trace(vec![
            span("early", ObservationType::Llm, 1.0),
            span("winner", ObservationType::Llm, 5.0),
            span("tie", ObservationType::Llm, 5.0),
            span("text", ObservationType::Llm, 7.0),
            span("missing", ObservationType::Llm, 8.0),
            span("tool", ObservationType::Tool, 9.0),
        ])),
        ..Reader::with_spans(&[
            ("early", Detail::chat("early")),
            ("winner", Detail::chat("latest")),
            ("tie", Detail::chat("wrong tie")),
            ("text", Detail::raw("raw", "answer")),
            ("tool", Detail::chat("not LLM")),
        ])
    };
    let result = build_cases(
        &request(vec![trace_source("")]),
        &reader,
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.cases[0].source.span_id, "winner");
    assert_eq!(
        reader.reads.into_inner().unwrap(),
        ["trace:t1", "t1:missing:ref", "t1:text:ref", "t1:winner:ref"]
    );
}

#[rstest]
#[case::negative_zero(-0.0)]
#[case::not_a_number(f64::NAN)]
#[tokio::test]
async fn floating_point_ties_preserve_trace_order(#[case] offset: f64) {
    let reader = Reader {
        trace: Some(trace(vec![
            span("first", ObservationType::Llm, offset),
            span("second", ObservationType::Llm, 0.0),
        ])),
        ..Reader::with_spans(&[
            ("first", Detail::chat("one")),
            ("second", Detail::chat("two")),
        ])
    };
    let result = build_cases(
        &request(vec![trace_source("")]),
        &reader,
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.cases[0].source.span_id, "first");
}

#[rstest]
#[case::missing_trace(None)]
#[case::no_conversation(Some(trace(vec![span("text",ObservationType::Llm,1.0)])))]
#[tokio::test]
async fn whole_trace_without_conversation_reports_no_content(
    #[case] trace: Option<litellm_traces::Trace>,
) {
    let reader = Reader {
        trace,
        ..Reader::with_spans(&[("text", Detail::raw("raw", "answer"))])
    };
    let result = build_cases(
        &request(vec![trace_source("")]),
        &reader,
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert!(result.cases.is_empty());
    assert_eq!(
        result.skipped,
        [SkippedCase {
            source: CaseSource {
                trace_id: "t1".into(),
                trace_ref: "ref".into(),
                ..Default::default()
            },
            reason: SkipReason::NoContent
        }]
    );
}

#[rstest]
#[tokio::test]
async fn skipped_sources_keep_provenance_and_unread_sources_do_not_fetch() {
    let reader = Reader::with_spans(&[
        ("big", Detail::chat(&"x".repeat(40))),
        ("s1", Detail::chat("one")),
        ("later", Detail::chat("two")),
    ]);
    let limits = Limits {
        max_cases: 2,
        max_case_chars: 30,
    };
    let existing = DatasetCase {
        included: false,
        ..case("existing")
    };
    let result = build_cases(
        &request(vec![
            trace_source("missing"),
            trace_source("big"),
            trace_source("s1"),
            trace_source("s1"),
            trace_source("later"),
        ]),
        &reader,
        &[existing],
        &Scope::default(),
        limits,
    )
    .await
    .unwrap();
    assert_eq!(result.cases.len(), 1);
    assert_eq!(
        result
            .skipped
            .iter()
            .map(|s| (s.source.span_id.as_str(), s.reason))
            .collect::<Vec<_>>(),
        [
            ("missing", SkipReason::NoContent),
            ("big", SkipReason::TooLarge),
            ("s1", SkipReason::OverLimit),
            ("later", SkipReason::OverLimit)
        ]
    );
    assert_eq!(
        reader.reads.into_inner().unwrap(),
        ["t1:missing:ref", "t1:big:ref", "t1:s1:ref"]
    );
}

#[rstest]
#[tokio::test]
async fn existing_hashes_and_duplicate_candidates_are_skipped() {
    let request = request(vec![text("one"), text("two"), text("one")]);
    let first = build_cases(
        &request,
        &Reader::default(),
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(first.cases.len(), 2);
    assert_eq!(first.skipped[0].reason, SkipReason::Duplicate);
    let second = build_cases(
        &request,
        &Reader::default(),
        &first.cases,
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert!(second.cases.is_empty());
    assert_eq!(
        second.skipped.iter().map(|s| s.reason).collect::<Vec<_>>(),
        [SkipReason::Duplicate; 3]
    );
}

#[rstest]
#[tokio::test]
async fn candidates_within_one_source_report_duplicates_before_capacity_and_keep_invalid_skips() {
    let lines = "{\"messages\":[{\"role\":\"user\",\"content\":\"one\"}]}\n{\"messages\":[{\"role\":\"user\",\"content\":\"one\"}]}\n{\"messages\":[{\"role\":\"user\",\"content\":\"two\"}]}\n{bad\n{\"messages\":[]}";
    let result = build_cases(
        &request(vec![text(lines)]),
        &Reader::default(),
        &[],
        &Scope::default(),
        Limits {
            max_cases: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(result.cases.len(), 1);
    assert_eq!(
        result.skipped.iter().map(|s| s.reason).collect::<Vec<_>>(),
        [
            SkipReason::Duplicate,
            SkipReason::OverLimit,
            SkipReason::Invalid,
            SkipReason::NoContent
        ]
    );
}

fn finding_source() -> BuildSource {
    BuildSource::Finding(FindingSource {
        lens_id: "lens".into(),
        finding_ids: vec!["f1".into(), "f2".into()],
    })
}

#[rstest]
#[case::negative(-1)]
#[case::zero(0)]
#[tokio::test]
async fn full_dataset_reports_each_source_without_reading(#[case] max_cases: i64) {
    let reader = Reader::default();
    let result = build_cases(
        &request(vec![trace_source("s"), finding_source(), text("q")]),
        &reader,
        &[],
        &Scope::default(),
        Limits {
            max_cases,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(result.cases.is_empty());
    assert_eq!(
        result.skipped,
        [
            SkippedCase {
                source: CaseSource {
                    trace_id: "t1".into(),
                    trace_ref: "ref".into(),
                    span_id: "s".into(),
                    ..Default::default()
                },
                reason: SkipReason::OverLimit
            },
            SkippedCase {
                source: CaseSource {
                    lens_id: "lens".into(),
                    finding_id: "f1".into(),
                    ..Default::default()
                },
                reason: SkipReason::OverLimit
            },
            SkippedCase {
                source: CaseSource::default(),
                reason: SkipReason::OverLimit
            }
        ]
    );
    assert!(reader.reads.into_inner().unwrap().is_empty());
}

fn execution(parts: Value) -> String {
    base64::engine::general_purpose::URL_SAFE.encode(parts.to_string())
}

#[rstest]
#[tokio::test]
async fn evidence_deduplicates_by_execution_and_span_and_retains_first_finding() {
    let first = execution(json!(["traces", "alpha", "t1", "ref1"]));
    let second = execution(json!(["traces", "alpha", "t1"]));
    let reader = Reader {
        stored: vec![
            Finding {
                id: "f1".into(),
                evidence: vec![Evidence {
                    execution_id: first.clone(),
                    span_id: "s1".into(),
                }],
            },
            Finding {
                id: "f2".into(),
                evidence: vec![
                    Evidence {
                        execution_id: first,
                        span_id: "s1".into(),
                    },
                    Evidence {
                        execution_id: second,
                        span_id: "s2".into(),
                    },
                ],
            },
        ],
        ..Reader::with_spans(&[("s1", Detail::chat("one")), ("s2", Detail::chat("two"))])
    };
    let result = build_cases(
        &request(vec![finding_source()]),
        &reader,
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        result
            .cases
            .iter()
            .map(|c| c.source.clone())
            .collect::<Vec<_>>(),
        [
            CaseSource {
                trace_id: "t1".into(),
                trace_ref: "ref1".into(),
                span_id: "s1".into(),
                finding_id: "f1".into(),
                lens_id: "lens".into()
            },
            CaseSource {
                trace_id: "t1".into(),
                trace_ref: "".into(),
                span_id: "s2".into(),
                finding_id: "f2".into(),
                lens_id: "lens".into()
            }
        ]
    );
    assert_eq!(
        reader.reads.into_inner().unwrap(),
        ["finding:lens", "t1:s1:ref1", "t1:s2:"]
    );
}

#[rstest]
#[case::legacy(execution(json!(["traces","alpha","t1"])), "")]
#[case::current(execution(json!(["traces","alpha","t1","ref1"])), "ref1")]
#[case::ignored_ascii(format!(" {}\n$",execution(json!(["traces","alpha","t1","ref1"]))), "ref1")]
#[case::extra_padding(format!("{}====",execution(json!(["traces","alpha","t1","ref1"]))), "ref1")]
#[case::interior_padding("WyJ0=cmFjZXMiLCJ0ZWFtIiwidDEiLCJyZWYxIl0=".into(), "ref1")]
#[case::padding_after_one_sextet("WyJ0cmFjZ===XMiLCJ0ZWFtIiwidDEiLCJyZWYxIl0=".into(), "ref1")]
#[case::terminal_padding_ignores_suffix("WyJ0cmFjZXMiLCJ0ZWFtIiwidDEiLCJyZWYxIl0=ignored".into(), "ref1")]
#[tokio::test]
async fn execution_ids_keep_legacy_and_permissive_decoding(
    #[case] id: String,
    #[case] trace_ref: &str,
) {
    let reader = Reader {
        stored: vec![Finding {
            id: "f1".into(),
            evidence: vec![Evidence {
                execution_id: id,
                span_id: "s1".into(),
            }],
        }],
        ..Reader::with_spans(&[("s1", Detail::chat("q"))])
    };
    let result = build_cases(
        &request(vec![finding_source()]),
        &reader,
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        (
            result.cases[0].source.trace_id.as_str(),
            result.cases[0].source.trace_ref.as_str()
        ),
        ("t1", trace_ref)
    );
}

#[rstest]
#[case::undecodable("not-an-execution".into())]
#[case::short_tuple(execution(json!(["traces","alpha"])))]
#[case::long_tuple(execution(json!(["traces","alpha","t1","ref","extra"])))]
#[case::nonstring(execution(json!(["traces","alpha",1])))]
#[case::unicode("é".into())]
#[case::invalid_json(base64::engine::general_purpose::URL_SAFE.encode("not JSON"))]
#[case::missing_padding("WyJ0cmFjZXMiLCJ0ZWFtIiwidDEiLCJyZWYxIl0".into())]
#[case::incomplete_padding("WyJ0cmFjZXMiLCJhbHBoYSIsInQxIiwicmVmIl0gIA=".into())]
#[tokio::test]
async fn invalid_execution_ids_skip_without_reading_a_span(#[case] id: String) {
    let reader = Reader {
        stored: vec![Finding {
            id: "f1".into(),
            evidence: vec![Evidence {
                execution_id: id,
                span_id: "s1".into(),
            }],
        }],
        ..Default::default()
    };
    let result = build_cases(
        &request(vec![finding_source()]),
        &reader,
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert!(result.cases.is_empty());
    assert_eq!(
        result.skipped,
        [SkippedCase {
            source: CaseSource {
                span_id: "s1".into(),
                finding_id: "f1".into(),
                lens_id: "lens".into(),
                ..Default::default()
            },
            reason: SkipReason::NoContent
        }]
    );
    assert_eq!(reader.reads.into_inner().unwrap(), ["finding:lens"]);
}

#[rstest]
#[tokio::test]
async fn missing_evidence_span_retains_decoded_trace_location() {
    let reader = Reader {
        stored: vec![Finding {
            id: "f1".into(),
            evidence: vec![Evidence {
                execution_id: execution(json!(["traces", "alpha", "t1", "ref1"])),
                span_id: "gone".into(),
            }],
        }],
        ..Default::default()
    };
    let result = build_cases(
        &request(vec![finding_source()]),
        &reader,
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        result.skipped,
        [SkippedCase {
            source: CaseSource {
                trace_id: "t1".into(),
                trace_ref: "ref1".into(),
                span_id: "gone".into(),
                finding_id: "f1".into(),
                lens_id: "lens".into()
            },
            reason: SkipReason::NoContent
        }]
    );
}

#[rstest]
#[case::trace(trace_source(""))]
#[case::span(trace_source("s"))]
#[case::findings(finding_source())]
#[tokio::test]
async fn reader_errors_propagate_without_partial_success(#[case] source: BuildSource) {
    let reader = Reader {
        failure: true,
        ..Default::default()
    };
    let result = build_cases(
        &request(vec![text("okay"), source]),
        &reader,
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await;
    assert!(matches!(result, Err(ReadError::LensNotFound)));
}

#[rstest]
#[tokio::test]
async fn jsonl_duplicate_fields_keep_the_last_value_at_every_level() {
    let input = r#"{"messages":[],"messages":[{"role":"tool","role":"user","content":"first","content":"last","tool_calls":[{"name":"first","name":"last","arguments":"{}"}]}],"reply":"first","reply":"last","source":{"span_id":"first","span_id":"last"}}"#;
    let result = build_cases(
        &request(vec![text(input)]),
        &Reader::default(),
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.cases.len(), 1);
    assert_eq!(
        result.cases[0].messages[0].role,
        lens_contract::datasets::DatasetRole::User
    );
    assert_eq!(result.cases[0].messages[0].content, "last");
    assert_eq!(result.cases[0].messages[0].tool_calls[0].name, "last");
    assert_eq!(result.cases[0].reply, "last");
    assert_eq!(result.cases[0].source.span_id, "last");
}

#[rstest]
#[tokio::test]
async fn execution_ids_accept_both_url_safe_characters_and_utf8() {
    let reader = Reader {
        stored: vec![Finding {
            id: "f1".into(),
            evidence: vec![Evidence {
                execution_id: "WyJ0cmFjZXMiLCJhbHBoYSIsIs-_IiwicmVmIl0=".into(),
                span_id: "s1".into(),
            }],
        }],
        ..Default::default()
    };
    let result = build_cases(
        &request(vec![finding_source()]),
        &reader,
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.skipped[0].source.trace_id, "Ͽ");
    assert_eq!(result.skipped[0].source.trace_ref, "ref");
}

#[rstest]
#[case::nan("NaN")]
#[case::positive_infinity("Infinity")]
#[case::negative_infinity("-Infinity")]
#[case::float_overflow("1e999")]
#[case::large_integer("999999999999999999999999999999999999999999999999999")]
#[tokio::test]
async fn jsonl_ignores_numeric_metadata_without_coercing_numeric_record_fields(
    #[case] number: &str,
) {
    let ignored = format!(r#"{{"messages":[],"reply":"answer","ignored":{{"value":{number}}}}}"#);
    let invalid = format!(r#"{{"messages":[{{"role":"user","content":{number}}}]}}"#);
    let result = build_cases(
        &request(vec![text(&format!("{ignored}\n{invalid}"))]),
        &Reader::default(),
        &[],
        &Scope::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.cases.len(), 1);
    assert_eq!(result.cases[0].reply, "answer");
    assert_eq!(
        result.skipped,
        [SkippedCase {
            source: CaseSource::default(),
            reason: SkipReason::Invalid
        }]
    );
}
