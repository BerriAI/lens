use lens_contract::worker;
use rstest::{fixture, rstest};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Roundtrip {
    name: String,
    case: String,
    input: Value,
    result: Value,
}

#[fixture]
fn fixtures() -> Vec<Roundtrip> {
    serde_json::from_str(include_str!("fixtures/worker-roundtrips-v7.json")).unwrap()
}

fn roundtrip<T: DeserializeOwned + Serialize>(input: Value) -> Value {
    match serde_json::from_value::<T>(input) {
        Ok(value) => json!({"ok": serde_json::to_value(value).unwrap()}),
        Err(error) => json!({"error": error.to_string()}),
    }
}

#[rstest]
#[case::activity("Activity", roundtrip::<worker::Activity>, 7)]
#[case::agent_test_case("AgentTestCase", roundtrip::<worker::AgentTestCase>, 5)]
#[case::candidate("Candidate", roundtrip::<worker::Candidate>, 7)]
#[case::catalog_entry("CatalogEntry", roundtrip::<worker::CatalogEntry>, 7)]
#[case::check("Check", roundtrip::<worker::Check>, 5)]
#[case::checkpoint("Checkpoint", roundtrip::<worker::Checkpoint>, 4)]
#[case::claim("Claim", roundtrip::<worker::Claim>, 6)]
#[case::clusters("Clusters", roundtrip::<worker::Clusters>, 3)]
#[case::coverage("Coverage", roundtrip::<worker::Coverage>, 3)]
#[case::evidence("Evidence", roundtrip::<worker::Evidence>, 6)]
#[case::evidence_reply("EvidenceReply", roundtrip::<worker::EvidenceReply>, 4)]
#[case::evidence_request("EvidenceRequest", roundtrip::<worker::EvidenceRequest>, 4)]
#[case::execution("Execution", roundtrip::<worker::Execution>, 10)]
#[case::execution_content("ExecutionContent", roundtrip::<worker::ExecutionContent>, 5)]
#[case::extraction("Extraction", roundtrip::<worker::Extraction>, 3)]
#[case::finding("Finding", roundtrip::<worker::Finding>, 11)]
#[case::finding_draft("FindingDraft", roundtrip::<worker::FindingDraft>, 7)]
#[case::finding_group("FindingGroup", roundtrip::<worker::FindingGroup>, 5)]
#[case::finding_groups("FindingGroups", roundtrip::<worker::FindingGroups>, 4)]
#[case::findings("Findings", roundtrip::<worker::Findings>, 3)]
#[case::in_flight("InFlight", roundtrip::<worker::InFlight>, 7)]
#[case::issue_brief("IssueBrief", roundtrip::<worker::IssueBrief>, 7)]
#[case::job("Job", roundtrip::<worker::Job>, 9)]
#[case::lens_settings("LensSettings", roundtrip::<worker::LensSettings>, 5)]
#[case::metadata_filter("MetadataFilter", roundtrip::<worker::MetadataFilter>, 5)]
#[case::model_message("ModelMessage", roundtrip::<worker::ModelMessage>, 5)]
#[case::model_request("ModelRequest", roundtrip::<worker::ModelRequest>, 5)]
#[case::model_result("ModelResult", roundtrip::<worker::ModelResult>, 5)]
#[case::observation("Observation", roundtrip::<worker::Observation>, 5)]
#[case::progress("Progress", roundtrip::<worker::Progress>, 3)]
#[case::python_agent_turn_extraction("PythonAgentTurnExtraction", roundtrip::<worker::PythonAgentTurnExtraction>, 3)]
#[case::python_agent_turn_findings("PythonAgentTurnFindings", roundtrip::<worker::PythonAgentTurnFindings>, 3)]
#[case::python_request("PythonRequest", roundtrip::<worker::PythonRequest>, 5)]
#[case::result("Result", roundtrip::<worker::Result>, 4)]
#[case::review("Review", roundtrip::<worker::Review>, 10)]
#[case::review_index("ReviewIndex", roundtrip::<worker::ReviewIndex>, 6)]
#[case::review_record("ReviewRecord", roundtrip::<worker::ReviewRecord>, 6)]
#[case::review_span("ReviewSpan", roundtrip::<worker::ReviewSpan>, 7)]
#[case::review_verdict("ReviewVerdict", roundtrip::<worker::ReviewVerdict>, 6)]
#[case::review_version("ReviewVersion", roundtrip::<worker::ReviewVersion>, 5)]
#[case::run_assessment("RunAssessment", roundtrip::<worker::RunAssessment>, 4)]
#[case::sample("Sample", roundtrip::<worker::Sample>, 5)]
#[case::step("Step", roundtrip::<worker::Step>, 6)]
#[case::tool_count("ToolCount", roundtrip::<worker::ToolCount>, 5)]
#[case::trace_part("TracePart", roundtrip::<worker::TracePart>, 8)]
fn preserves_generated_worker_serialization(
    fixtures: Vec<Roundtrip>,
    #[case] name: &str,
    #[case] decode: fn(Value) -> Value,
    #[case] count: usize,
) {
    let selected = fixtures
        .into_iter()
        .filter(|fixture| fixture.name == name)
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), count);
    for fixture in selected {
        assert_eq!(
            decode(fixture.input),
            fixture.result,
            "{name}: {}",
            fixture.case
        );
    }
}
