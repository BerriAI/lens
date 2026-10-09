use lens_contract::{error::ConversionError, worker};
use rstest::rstest;
use serde::{Serialize, de::DeserializeOwned};
use std::{fmt::Debug, ops::Deref, str::FromStr};

fn convert<T>(input: &str) -> Result<String, String>
where
    T: FromStr<Err = ConversionError>
        + for<'a> TryFrom<&'a str, Error = ConversionError>
        + for<'a> TryFrom<&'a String, Error = ConversionError>
        + TryFrom<String, Error = ConversionError>
        + Into<String>
        + Deref<Target = String>
        + DeserializeOwned
        + Serialize
        + PartialEq
        + Debug,
{
    let parsed = input.parse::<T>();
    let owned = input.to_owned();
    let deserialized = serde_json::from_value::<T>(serde_json::json!(input));
    match parsed {
        Ok(value) => {
            assert_eq!(T::try_from(input).unwrap(), value);
            assert_eq!(T::try_from(&owned).unwrap(), value);
            assert_eq!(T::try_from(owned).unwrap(), value);
            assert_eq!(deserialized.unwrap(), value);
            assert_eq!(value.deref(), input);
            assert_eq!(
                serde_json::to_value(&value).unwrap(),
                serde_json::json!(input)
            );
            Ok(value.into())
        }
        Err(error) => {
            assert_eq!(
                T::try_from(input).unwrap_err().to_string(),
                error.to_string()
            );
            assert_eq!(
                T::try_from(&owned).unwrap_err().to_string(),
                error.to_string()
            );
            assert_eq!(
                T::try_from(owned).unwrap_err().to_string(),
                error.to_string()
            );
            assert_eq!(deserialized.unwrap_err().to_string(), error.to_string());
            Err(error.to_string())
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Boundary {
    Empty,
    BelowMinimum,
    Minimum,
    AboveMinimum,
    Maximum,
    AboveMaximum,
}

#[rstest]
#[case::agent_test_case_expected(convert::<worker::AgentTestCaseExpected>, 1, None)]
#[case::agent_test_case_input(convert::<worker::AgentTestCaseInput>, 1, None)]
#[case::check_id(convert::<worker::CheckId>, 1, None)]
#[case::check_instruction(convert::<worker::CheckInstruction>, 3, None)]
#[case::checkpoint_working_notes(convert::<worker::CheckpointWorkingNotes>, 1, None)]
#[case::evidence_quote(convert::<worker::EvidenceQuote>, 1, None)]
#[case::extraction_reasoning(convert::<worker::ExtractionReasoning>, 0, Some(800))]
#[case::finding_description(convert::<worker::FindingDescription>, 10, None)]
#[case::finding_draft_description(convert::<worker::FindingDraftDescription>, 10, None)]
#[case::finding_draft_title(convert::<worker::FindingDraftTitle>, 3, None)]
#[case::finding_title(convert::<worker::FindingTitle>, 3, None)]
#[case::issue_brief_problem(convert::<worker::IssueBriefProblem>, 10, None)]
#[case::issue_brief_user_goal(convert::<worker::IssueBriefUserGoal>, 3, None)]
#[case::issue_brief_what_happened(convert::<worker::IssueBriefWhatHappened>, 3, None)]
#[case::lens_settings_model(convert::<worker::LensSettingsModel>, 1, None)]
#[case::lens_settings_name(convert::<worker::LensSettingsName>, 1, None)]
#[case::metadata_filter_key(convert::<worker::MetadataFilterKey>, 1, None)]
#[case::metadata_filter_value(convert::<worker::MetadataFilterValue>, 1, None)]
#[case::model_request_prompt(convert::<worker::ModelRequestPrompt>, 1, None)]
#[case::python_agent_turn_extraction_checkpoint(convert::<worker::PythonAgentTurnExtractionCheckpoint>, 1, None)]
#[case::python_agent_turn_findings_checkpoint(convert::<worker::PythonAgentTurnFindingsCheckpoint>, 1, None)]
#[case::python_request_code(convert::<worker::PythonRequestCode>, 1, None)]
#[case::review_reasoning(convert::<worker::ReviewReasoning>, 0, Some(800))]
#[case::review_span_kind(convert::<worker::ReviewSpanKind>, 0, Some(40))]
#[case::review_span_name(convert::<worker::ReviewSpanName>, 0, Some(120))]
#[case::review_span_preview(convert::<worker::ReviewSpanPreview>, 0, Some(240))]
#[case::review_verdict_summary(convert::<worker::ReviewVerdictSummary>, 0, Some(300))]
#[case::step_label(convert::<worker::StepLabel>, 0, Some(200))]
#[case::step_model(convert::<worker::StepModel>, 0, Some(200))]
#[case::step_purpose(convert::<worker::StepPurpose>, 0, Some(40))]
fn string_constraints_preserve_unicode_character_lengths(
    #[case] convert: fn(&str) -> Result<String, String>,
    #[case] minimum: usize,
    #[case] maximum: Option<usize>,
    #[values(
        Boundary::Empty,
        Boundary::BelowMinimum,
        Boundary::Minimum,
        Boundary::AboveMinimum,
        Boundary::Maximum,
        Boundary::AboveMaximum
    )]
    boundary: Boundary,
    #[values('a', '🦀')] character: char,
) {
    let length = match boundary {
        Boundary::Empty => 0,
        Boundary::BelowMinimum => minimum.saturating_sub(1),
        Boundary::Minimum => minimum,
        Boundary::AboveMinimum => minimum + 1,
        Boundary::Maximum => maximum.unwrap_or(1000),
        Boundary::AboveMaximum => maximum.unwrap_or(1000) + 1,
    };
    let input = character.to_string().repeat(length);
    let expected = if length < minimum {
        Err(format!("shorter than {minimum} characters"))
    } else if let Some(maximum) = maximum
        && length > maximum
    {
        Err(format!("longer than {maximum} characters"))
    } else {
        Ok(input.clone())
    };
    assert_eq!(convert(&input), expected);
}
