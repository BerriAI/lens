use lens_evals_sdk::reporting::safe_text;
use rstest::rstest;

#[rstest]
#[case::link(
    "[click](https://attacker.example)",
    r"\[click\](https://attacker.example)"
)]
#[case::escaped_link(
    r"\[click\](https://attacker.example)",
    r"\\\[click\\\](https://attacker.example)"
)]
#[case::backslash(r"path\file", r"path\\file")]
fn report_text_cannot_reactivate_escaped_markdown(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(safe_text(text), expected);
}
