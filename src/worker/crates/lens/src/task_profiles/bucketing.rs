use std::{collections::BTreeMap, time::Duration};

use lens_contract::{profiles::Taxonomy, signals::SignalStep};
use lens_signals::{DecisionRequest, Decisions, Question, SignalState};
use serde::Deserialize;
use serde_json::Value;

use crate::error::DiscoveryError;

const TASK: &str = "task";
const FIT: &str = "fit";
const NONE: &str = "none";
const MIN_FIT: f64 = 0.5;
const INPUT_CHARACTERS: usize = 1_500;
// Choice supports 255 options including none: https://docs.typesafe.ai/primitives/choice (2026-11-06).
pub const MAX_OPTIONS: usize = 254;
pub const TAG: &str = "litellm-lens-profiles";
/// Providers that serve Jev's Choice and Noul primitives. A generative chat judge is never used.
pub const DECISION_PROVIDERS: [&str; 2] = ["typesafe", "decisions_compatible"];

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    Assigned(String),
    Unknown,
}

fn option(index: usize) -> String {
    format!("T{:03}", index + 1)
}

fn criterion(task: &lens_contract::profiles::TaskDefinition) -> String {
    [
        task.name.as_str(),
        task.description.as_str(),
        &format!("Belongs when: {}", task.inclusion_criteria),
        &format!("Does not belong: {}", task.exclusion_criteria),
    ]
    .into_iter()
    .filter(|part| !part.trim().is_empty() && !part.ends_with(": "))
    .collect::<Vec<_>>()
    .join(" ")
}

fn definitions(taxonomy: &Taxonomy) -> String {
    taxonomy
        .tasks
        .iter()
        .map(|task| format!("({}) {}", task.name, criterion(task)))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn request(
    model: &str,
    taxonomy: &Taxonomy,
    input: &str,
) -> Result<DecisionRequest, DiscoveryError> {
    if taxonomy.tasks.is_empty() || taxonomy.tasks.len() > MAX_OPTIONS {
        return Err(DiscoveryError::InvalidReply("taxonomy"));
    }
    let criteria = taxonomy
        .tasks
        .iter()
        .enumerate()
        .map(|(index, task)| (option(index), Some(criterion(task))))
        .chain([(
            NONE.to_owned(),
            Some("No listed task definition fits this request".to_owned()),
        )])
        .collect();
    Ok(DecisionRequest {
        model: model.to_owned(),
        state: SignalState {
            task: "A request someone sent to an AI agent. Judge only the request text.",
            steps: vec![SignalStep {
                kind: "user".into(),
                name: "request".into(),
                content: input.chars().take(INPUT_CHARACTERS).collect(),
            }],
        },
        questions: BTreeMap::from([
            (
                TASK.to_owned(),
                Question::Choice {
                    instructions: "Which task definition describes the kind of work this request asks for? Select none when no definition fits.".into(),
                    criteria,
                },
            ),
            (
                FIT.to_owned(),
                Question::Noul {
                    instructions: format!(
                        "Does this request clearly ask for one of these kinds of work rather than something outside them? {}",
                        definitions(taxonomy)
                    ),
                },
            ),
        ]),
        timeout: Duration::from_secs(60),
        tags: vec![TAG],
    })
}

#[derive(Deserialize)]
struct Output {
    answers: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Answer {
    Choice { choice: String, confidence: f64 },
    Noul { noul: f64 },
}

fn answer(output: &Output, key: &str) -> Option<Answer> {
    serde_json::from_value(output.answers.get(key)?.clone()).ok()
}

/// Unknown when Jev selects none or doubts the fit; an unparsable reply is an error, not a guess.
pub fn decision(taxonomy: &Taxonomy, reply: Value) -> Result<Decision, DiscoveryError> {
    let output: Output =
        serde_json::from_value(reply).map_err(|_| DiscoveryError::InvalidReply("decision"))?;
    let Some(Answer::Choice { choice, confidence }) = answer(&output, TASK) else {
        return Err(DiscoveryError::InvalidReply("decision"));
    };
    let Some(Answer::Noul { noul }) = answer(&output, FIT) else {
        return Err(DiscoveryError::InvalidReply("decision"));
    };
    if !(0.0..=1.0).contains(&confidence) || !(0.0..=1.0).contains(&noul) {
        return Err(DiscoveryError::InvalidReply("decision"));
    }
    if choice == NONE || noul < MIN_FIT {
        return Ok(Decision::Unknown);
    }
    taxonomy
        .tasks
        .iter()
        .enumerate()
        .find(|(index, _)| option(*index) == choice)
        .map(|(_, task)| Decision::Assigned(task.task_id.clone()))
        .ok_or(DiscoveryError::InvalidReply("decision"))
}

pub async fn decide(
    decisions: &impl Decisions,
    model: &str,
    taxonomy: &Taxonomy,
    input: &str,
) -> Result<Decision, DiscoveryError> {
    let reply = decisions
        .complete(&request(model, taxonomy, input)?)
        .await
        .map_err(|_| DiscoveryError::Provider("decision"))?;
    decision(taxonomy, reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_contract::profiles::{TaskDefinition, TaxonomyModels, TaxonomyStatus};
    use rstest::rstest;
    use serde_json::json;

    fn taxonomy() -> Taxonomy {
        Taxonomy {
            taxonomy_id: "tax".into(),
            agent: "agent".into(),
            version: 2,
            revision: 3,
            status: TaxonomyStatus::Frozen,
            sample: false,
            tasks: ["debug", "docs"]
                .into_iter()
                .map(|task_id| TaskDefinition {
                    task_id: task_id.into(),
                    name: task_id.into(),
                    description: String::new(),
                    inclusion_criteria: String::new(),
                    exclusion_criteria: String::new(),
                    required_capabilities: Vec::new(),
                    example_trace_ids: Vec::new(),
                })
                .collect(),
            points: Vec::new(),
            job: None,
            discovered_at: None,
            frozen_at: None,
            error: None,
            models: TaxonomyModels::default(),
            discovery: None,
            windows: Vec::new(),
        }
    }

    fn reply(choice: &str, noul: f64) -> Value {
        json!({"answers": {
            "task": {"type": "choice", "choice": choice, "confidence": 0.9},
            "fit": {"type": "noul", "noul": noul}
        }})
    }

    #[rstest]
    #[case::assigned(reply("T002", 0.9), Ok(Decision::Assigned("docs".into())))]
    #[case::none(reply("none", 0.9), Ok(Decision::Unknown))]
    #[case::low_fit(reply("T001", 0.2), Ok(Decision::Unknown))]
    #[case::unknown_option(reply("T009", 0.9), Err(DiscoveryError::InvalidReply("decision")))]
    #[case::missing_fit(json!({"answers": {"task": {"type": "choice", "choice": "T001", "confidence": 0.9}}}), Err(DiscoveryError::InvalidReply("decision")))]
    #[case::out_of_range(reply("T001", 1.5), Err(DiscoveryError::InvalidReply("decision")))]
    fn jev_replies_map_to_a_task_or_unknown(
        #[case] value: Value,
        #[case] expected: Result<Decision, DiscoveryError>,
    ) {
        assert_eq!(decision(&taxonomy(), value), expected);
    }

    #[rstest]
    fn every_frozen_task_and_none_are_offered() {
        let request = request("jev", &taxonomy(), "fix the login test").expect("request");

        let Question::Choice { criteria, .. } = &request.questions[TASK] else {
            panic!("task question is a choice");
        };
        assert_eq!(
            criteria.keys().map(String::as_str).collect::<Vec<_>>(),
            ["T001", "T002", "none"]
        );
        assert_eq!(request.tags, [TAG]);
    }

    #[rstest]
    fn the_fit_question_names_every_task_itself() {
        let request = request("jev", &taxonomy(), "fix the login test").expect("request");

        let Question::Noul { instructions } = &request.questions[FIT] else {
            panic!("fit question is a noul");
        };
        assert!(instructions.contains("(debug)") && instructions.contains("(docs)"));
    }
}
