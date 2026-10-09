use lens_contract::{error::ConversionError, worker};
use rstest::rstest;
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fmt::{Debug, Display},
    str::FromStr,
};

fn convert<T>(input: &str) -> Result<String, String>
where
    T: FromStr<Err = ConversionError>
        + for<'a> TryFrom<&'a str, Error = ConversionError>
        + for<'a> TryFrom<&'a String, Error = ConversionError>
        + TryFrom<String, Error = ConversionError>
        + Display
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
            assert_eq!(
                serde_json::to_value(&value).unwrap(),
                serde_json::json!(input)
            );
            Ok(value.to_string())
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
            assert!(deserialized.is_err());
            Err(error.to_string())
        }
    }
}

#[rstest]
#[case::activity_operations_item_model(convert::<worker::ActivityOperationsItem>, "model")]
#[case::activity_operations_item_read(convert::<worker::ActivityOperationsItem>, "read")]
#[case::activity_operations_item_search(convert::<worker::ActivityOperationsItem>, "search")]
#[case::activity_operations_item_python(convert::<worker::ActivityOperationsItem>, "python")]
#[case::activity_operations_item_catalog(convert::<worker::ActivityOperationsItem>, "catalog")]
#[case::activity_operations_item_review_catalog(convert::<worker::ActivityOperationsItem>, "review_catalog")]
#[case::activity_operations_item_read_reviews(convert::<worker::ActivityOperationsItem>, "read_reviews")]
#[case::activity_operations_item_search_reviews(convert::<worker::ActivityOperationsItem>, "search_reviews")]
#[case::activity_operations_item_history(convert::<worker::ActivityOperationsItem>, "history")]
#[case::activity_operations_item_checkpoint(convert::<worker::ActivityOperationsItem>, "checkpoint")]
#[case::activity_phase_load(convert::<worker::ActivityPhase>, "load")]
#[case::activity_phase_review(convert::<worker::ActivityPhase>, "review")]
#[case::activity_phase_group(convert::<worker::ActivityPhase>, "group")]
#[case::activity_phase_reconcile(convert::<worker::ActivityPhase>, "reconcile")]
#[case::activity_phase_investigate(convert::<worker::ActivityPhase>, "investigate")]
#[case::candidate_kind_issue(convert::<worker::CandidateKind>, "issue")]
#[case::candidate_kind_pattern(convert::<worker::CandidateKind>, "pattern")]
#[case::evidence_request_action_catalog(convert::<worker::EvidenceRequestAction>, "catalog")]
#[case::evidence_request_action_read(convert::<worker::EvidenceRequestAction>, "read")]
#[case::evidence_request_action_search(convert::<worker::EvidenceRequestAction>, "search")]
#[case::evidence_request_action_review_catalog(convert::<worker::EvidenceRequestAction>, "review_catalog")]
#[case::evidence_request_action_read_reviews(convert::<worker::EvidenceRequestAction>, "read_reviews")]
#[case::evidence_request_action_search_reviews(convert::<worker::EvidenceRequestAction>, "search_reviews")]
#[case::evidence_request_action_history(convert::<worker::EvidenceRequestAction>, "history")]
#[case::evidence_request_review_phase_initial(convert::<worker::EvidenceRequestReviewPhase>, "initial")]
#[case::evidence_request_review_phase_revisited(convert::<worker::EvidenceRequestReviewPhase>, "revisited")]
#[case::evidence_role_support(convert::<worker::EvidenceRole>, "support")]
#[case::evidence_role_counterexample(convert::<worker::EvidenceRole>, "counterexample")]
#[case::execution_source_traces(convert::<worker::ExecutionSource>, "traces")]
#[case::execution_source_requests(convert::<worker::ExecutionSource>, "requests")]
#[case::finding_draft_kind_issue(convert::<worker::FindingDraftKind>, "issue")]
#[case::finding_draft_kind_pattern(convert::<worker::FindingDraftKind>, "pattern")]
#[case::finding_draft_priority_high(convert::<worker::FindingDraftPriority>, "high")]
#[case::finding_draft_priority_medium(convert::<worker::FindingDraftPriority>, "medium")]
#[case::finding_draft_priority_low(convert::<worker::FindingDraftPriority>, "low")]
#[case::finding_kind_issue(convert::<worker::FindingKind>, "issue")]
#[case::finding_kind_pattern(convert::<worker::FindingKind>, "pattern")]
#[case::finding_priority_high(convert::<worker::FindingPriority>, "high")]
#[case::finding_priority_medium(convert::<worker::FindingPriority>, "medium")]
#[case::finding_priority_low(convert::<worker::FindingPriority>, "low")]
#[case::finding_status_open(convert::<worker::FindingStatus>, "open")]
#[case::finding_status_resolved(convert::<worker::FindingStatus>, "resolved")]
#[case::finding_status_dismissed(convert::<worker::FindingStatus>, "dismissed")]
#[case::job_status_queued(convert::<worker::JobStatus>, "queued")]
#[case::job_status_running(convert::<worker::JobStatus>, "running")]
#[case::job_status_completed(convert::<worker::JobStatus>, "completed")]
#[case::job_status_failed(convert::<worker::JobStatus>, "failed")]
#[case::job_status_cancelled(convert::<worker::JobStatus>, "cancelled")]
#[case::job_trigger_schedule(convert::<worker::JobTrigger>, "schedule")]
#[case::job_trigger_manual(convert::<worker::JobTrigger>, "manual")]
#[case::lens_settings_source_traces(convert::<worker::LensSettingsSource>, "traces")]
#[case::lens_settings_source_requests(convert::<worker::LensSettingsSource>, "requests")]
#[case::lens_settings_source_both(convert::<worker::LensSettingsSource>, "both")]
#[case::model_message_role_system(convert::<worker::ModelMessageRole>, "system")]
#[case::model_message_role_user(convert::<worker::ModelMessageRole>, "user")]
#[case::model_message_role_assistant(convert::<worker::ModelMessageRole>, "assistant")]
#[case::model_request_purpose_extract(convert::<worker::ModelRequestPurpose>, "extract")]
#[case::model_request_purpose_cluster(convert::<worker::ModelRequestPurpose>, "cluster")]
#[case::model_request_purpose_investigate(convert::<worker::ModelRequestPurpose>, "investigate")]
#[case::model_result_finish_reason_length(convert::<worker::ModelResultFinishReason>, "length")]
#[case::model_result_finish_reason_content_filter(convert::<worker::ModelResultFinishReason>, "content_filter")]
#[case::observation_kind_issue(convert::<worker::ObservationKind>, "issue")]
#[case::observation_kind_pattern(convert::<worker::ObservationKind>, "pattern")]
#[case::review_index_phase_initial(convert::<worker::ReviewIndexPhase>, "initial")]
#[case::review_index_phase_revisited(convert::<worker::ReviewIndexPhase>, "revisited")]
#[case::review_record_phase_initial(convert::<worker::ReviewRecordPhase>, "initial")]
#[case::review_record_phase_revisited(convert::<worker::ReviewRecordPhase>, "revisited")]
#[case::review_verdict_kind_issue(convert::<worker::ReviewVerdictKind>, "issue")]
#[case::review_verdict_kind_pattern(convert::<worker::ReviewVerdictKind>, "pattern")]
#[case::step_kind_stage(convert::<worker::StepKind>, "stage")]
#[case::step_kind_model(convert::<worker::StepKind>, "model")]
#[case::step_kind_error(convert::<worker::StepKind>, "error")]
#[case::tool_count_name_model(convert::<worker::ToolCountName>, "model")]
#[case::tool_count_name_read(convert::<worker::ToolCountName>, "read")]
#[case::tool_count_name_search(convert::<worker::ToolCountName>, "search")]
#[case::tool_count_name_python(convert::<worker::ToolCountName>, "python")]
#[case::tool_count_name_catalog(convert::<worker::ToolCountName>, "catalog")]
#[case::tool_count_name_review_catalog(convert::<worker::ToolCountName>, "review_catalog")]
#[case::tool_count_name_read_reviews(convert::<worker::ToolCountName>, "read_reviews")]
#[case::tool_count_name_search_reviews(convert::<worker::ToolCountName>, "search_reviews")]
#[case::tool_count_name_history(convert::<worker::ToolCountName>, "history")]
#[case::tool_count_name_checkpoint(convert::<worker::ToolCountName>, "checkpoint")]
fn enum_values_roundtrip(#[case] convert: fn(&str) -> Result<String, String>, #[case] value: &str) {
    assert_eq!(convert(value), Ok(value.to_owned()));
    assert_eq!(convert("not-a-wire-value"), Err("invalid value".to_owned()));
}
