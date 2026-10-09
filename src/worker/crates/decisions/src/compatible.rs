use std::collections::BTreeMap;

use lens_contract::signals::NoulAnswer;
use lens_signals::DecisionRequest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Error;

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    input: String,
    questions: Vec<Predicate<'a>>,
    metadata: Metadata<'a>,
}

#[derive(Serialize)]
struct Predicate<'a> {
    r#type: &'static str,
    name: &'a str,
    instructions: &'a str,
}

#[derive(Serialize)]
struct Metadata<'a> {
    tags: &'a [&'static str],
}

#[derive(Deserialize)]
struct Response {
    answers: Vec<Answer>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Answer {
    Predicate { name: String, probability: f64 },
    Refusal { name: String },
}

pub(crate) fn request(model: &str, request: &DecisionRequest) -> Result<Value, Error> {
    Ok(serde_json::to_value(Request {
        model,
        input: serde_json::to_string(&request.state)?,
        questions: request
            .questions
            .iter()
            .map(|(name, question)| Predicate {
                r#type: "predicate",
                name,
                instructions: &question.instructions,
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
    for answer in parsed.answers {
        let name = match &answer {
            Answer::Predicate { name, .. } | Answer::Refusal { name } => name,
        };
        if !request.questions.contains_key(name) || !seen.insert(name.clone()) {
            return Err(Error::InvalidAnswers);
        }
        if let Answer::Predicate { name, probability } = answer {
            if !(0.0..=1.0).contains(&probability) {
                return Err(Error::InvalidAnswers);
            }
            answers.insert(name, NoulAnswer::Noul { noul: probability });
        }
    }
    response["answers"] = serde_json::to_value(answers)?;
    Ok(response)
}
