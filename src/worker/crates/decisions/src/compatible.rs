use std::collections::BTreeMap;

use lens_contract::signals::NoulAnswer;
use lens_signals::{DecisionRequest, Question};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Error;

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    input: String,
    questions: Vec<GatewayQuestion<'a>>,
    metadata: Metadata<'a>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum GatewayQuestion<'a> {
    Predicate {
        name: &'a str,
        instructions: &'a str,
    },
    Choice {
        name: &'a str,
        instructions: &'a str,
        choices: Vec<Choice<'a>>,
    },
}

#[derive(Serialize)]
struct Choice<'a> {
    value: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
}

#[derive(Serialize)]
struct Metadata<'a> {
    tags: &'a [&'static str],
}

#[derive(Deserialize)]
struct Response {
    answers: Vec<NamedAnswer>,
}

#[derive(Deserialize)]
struct NamedAnswer {
    name: String,
    #[serde(flatten)]
    content: Value,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Answer {
    Predicate { probability: f64 },
    Choice { choice: String, confidence: f64 },
    Refusal,
}

pub(crate) fn request(model: &str, request: &DecisionRequest) -> Result<Value, Error> {
    Ok(serde_json::to_value(Request {
        model,
        input: serde_json::to_string(&request.state)?,
        questions: request
            .questions
            .iter()
            .map(|(name, question)| match question {
                Question::Noul { instructions } => {
                    GatewayQuestion::Predicate { name, instructions }
                }
                Question::Choice {
                    instructions,
                    criteria,
                } => GatewayQuestion::Choice {
                    name,
                    instructions,
                    choices: criteria
                        .iter()
                        .map(|(value, description)| Choice {
                            value,
                            description: description.as_deref(),
                        })
                        .collect(),
                },
            })
            .collect(),
        metadata: Metadata {
            tags: &request.tags,
        },
    })?)
}

pub(crate) fn response(mut response: Value, request: &DecisionRequest) -> Result<Value, Error> {
    let parsed: Response = serde_json::from_value(response.clone())?;
    let mut seen = std::collections::BTreeSet::new();
    let mut answers = BTreeMap::new();
    for NamedAnswer { name, content } in parsed.answers {
        let Some(question) = request.questions.get(&name) else {
            return Err(Error::InvalidAnswers);
        };
        if !seen.insert(name.clone()) {
            return Err(Error::InvalidAnswers);
        }
        let answer = match serde_json::from_value::<Answer>(content) {
            Ok(answer) => answer,
            Err(_) if matches!(question, Question::Choice { .. }) => continue,
            Err(error) => return Err(error.into()),
        };
        match (question, answer) {
            (Question::Noul { .. }, Answer::Predicate { probability }) => {
                if !(0.0..=1.0).contains(&probability) {
                    return Err(Error::InvalidAnswers);
                }
                answers.insert(
                    name,
                    serde_json::to_value(NoulAnswer::Noul { noul: probability })?,
                );
            }
            (Question::Choice { criteria, .. }, Answer::Choice { choice, confidence }) => {
                if criteria.contains_key(&choice) && (0.0..=1.0).contains(&confidence) {
                    answers.insert(name, serde_json::json!({"type":"choice","choice":choice,"confidence":confidence}));
                }
            }
            (_, Answer::Refusal) | (Question::Choice { .. }, _) => {}
            _ => return Err(Error::InvalidAnswers),
        }
    }
    response["answers"] = serde_json::to_value(answers)?;
    Ok(response)
}
