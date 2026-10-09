use base64::{Engine, engine::general_purpose::URL_SAFE};
use lens_contract::execution::ExecutionId;
use rstest::rstest;

#[rstest]
#[case::plain("team", r#"["traces", "team", "trace", "ref"]"#)]
#[case::unicode("é😀", r#"["traces", "\u00e9\ud83d\ude00", "trace", "ref"]"#)]
#[case::escapes(
    "\"\\\n\r\t\u{8}\u{c}\u{1}\u{7f}",
    r#"["traces", "\"\\\n\r\t\b\f\u0001\u007f", "trace", "ref"]"#
)]
fn encoded_identity_preserves_python_bytes(#[case] team: &str, #[case] expected: &str) {
    let identity = ExecutionId {
        source: "traces".into(),
        team_id: team.into(),
        trace_id: "trace".into(),
        trace_ref: "ref".into(),
    };
    assert_eq!(
        URL_SAFE.decode(identity.encode()).unwrap(),
        expected.as_bytes()
    );
    assert_eq!(ExecutionId::decode(&identity.encode()).unwrap(), identity);
}

#[rstest]
#[case::legacy(r#"["traces", "team", "trace"]"#, "traces\0team\0trace")]
#[case::empty_reference(r#"["traces", "team", "trace", ""]"#, "traces\0team\0trace")]
#[case::reference(r#"["requests", "team", "trace", "ref"]"#, "requests\0team\0ref")]
fn selection_preserves_legacy_identity_and_prefers_reference(
    #[case] value: &str,
    #[case] key: &str,
) {
    assert_eq!(
        ExecutionId::decode(&URL_SAFE.encode(value))
            .unwrap()
            .selection_key(),
        key
    );
}

#[rstest]
#[case::wrong_arity(r#"["traces", "team"]"#)]
#[case::wrong_type(r#"["traces", "team", 12]"#)]
#[case::invalid_json("invalid")]
fn malformed_decoded_identity_is_rejected(#[case] value: &str) {
    assert!(ExecutionId::decode(&URL_SAFE.encode(value)).is_none());
}
