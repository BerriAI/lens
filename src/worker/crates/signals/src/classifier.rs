use std::{collections::BTreeMap, future::Future, time::Duration};

use lens_contract::{
    investigations::Scope,
    signals::{NoulAnswer, SignalAttempt, SignalAttemptStatus, SignalConfig, SignalStep},
    worker::Execution,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{DecisionsError, Error, SignalReader};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SignalState {
    pub task: &'static str,
    pub steps: Vec<SignalStep>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Question {
    pub r#type: &'static str,
    pub instructions: String,
}

#[derive(Clone, Debug)]
pub struct DecisionRequest {
    pub model: String,
    pub state: SignalState,
    pub questions: BTreeMap<String, Question>,
    pub timeout: Duration,
    pub tags: Vec<&'static str>,
}

pub trait Decisions: Send + Sync {
    fn complete(
        &self,
        request: &DecisionRequest,
    ) -> impl Future<Output = Result<Value, DecisionsError>> + Send;
}

fn excerpt(content: &str) -> String {
    let length = content.chars().count();
    if length <= 2_000 {
        return content.into();
    }
    let head = content.chars().take(800).collect::<String>();
    let tail = content.chars().skip(length - 1_200).collect::<String>();
    format!(
        "{head}\n[... {} characters omitted ...]\n{tail}",
        length - 2_000
    )
}

fn take(
    steps: impl Iterator<Item = SignalStep>,
    mut remaining: usize,
    tail: bool,
) -> Vec<SignalStep> {
    let mut result = Vec::new();
    for step in steps {
        if remaining == 0 {
            break;
        }
        let length = step.content.chars().count();
        let used = remaining.min(length);
        let content = if tail {
            step.content.chars().skip(length - used).collect()
        } else {
            step.content.chars().take(used).collect()
        };
        result.push(SignalStep { content, ..step });
        remaining -= used;
    }
    result
}

fn bounded(steps: Vec<SignalStep>) -> Vec<SignalStep> {
    if steps
        .iter()
        .map(|step| step.content.chars().count())
        .sum::<usize>()
        <= 40_000
    {
        return steps;
    }
    let head = take(steps.iter().cloned(), 15_000, false);
    let mut tail = take(steps.iter().rev().cloned(), 25_000, true);
    tail.reverse();
    let omitted = steps.len() as i64 - head.len() as i64 - tail.len() as i64;
    head.into_iter()
        .chain([SignalStep {
            kind: "omitted".into(),
            name: String::new(),
            content: format!("{omitted} steps omitted"),
        }])
        .chain(tail)
        .collect()
}

pub async fn signal_state(
    reader: &impl SignalReader,
    scope: &Scope,
    execution: &Execution,
) -> Result<SignalState, Error> {
    let mut steps = Vec::new();
    let mut cursor = String::new();
    for _ in 0..3 {
        let content = reader.content(scope, execution, &cursor).await?;
        steps.extend(content.parts.into_iter().map(|part| SignalStep {
            kind: part.kind,
            name: part.name,
            content: excerpt(&part.content),
        }));
        let Some(next) = content.next_cursor else {
            break;
        };
        cursor = next;
    }
    Ok(SignalState {
        task: "An AI agent run recorded as a trace. Judge only what the user and the agent said and did in these steps.",
        steps: bounded(steps),
    })
}

fn score(value: &Value) -> Option<f64> {
    serde_json::from_value::<NoulAnswer>(value.clone())
        .ok()
        .map(|NoulAnswer::Noul { noul }| noul)
}

#[derive(Deserialize)]
struct Output {
    answers: BTreeMap<String, Value>,
}

async fn attempt(
    reader: &impl SignalReader,
    completion: &impl Decisions,
    scope: &Scope,
    execution: &Execution,
    config: &SignalConfig,
) -> Result<SignalAttempt, Error> {
    let request = DecisionRequest {
        model: config.model.clone(),
        state: signal_state(reader, scope, execution).await?,
        questions: config
            .signals
            .iter()
            .map(|signal| {
                (
                    signal.id.clone(),
                    Question {
                        r#type: "noul",
                        instructions: signal.question.clone(),
                    },
                )
            })
            .collect(),
        timeout: Duration::from_secs(60),
        tags: vec!["litellm-lens-signals"],
    };
    let response: serde_json::Map<String, Value> =
        serde_json::from_value(completion.complete(&request).await?).map_err(Error::Response)?;
    let response: Output =
        serde_json::from_value(Value::Object(response)).map_err(Error::Response)?;
    let scores = config
        .signals
        .iter()
        .filter_map(|signal| {
            response
                .answers
                .get(&signal.id)
                .and_then(score)
                .map(|score| (signal.id.clone(), score))
        })
        .collect::<BTreeMap<_, _>>();
    let complete = scores.len() == config.signals.len();
    Ok(SignalAttempt {
        status: if complete {
            SignalAttemptStatus::Classified
        } else {
            SignalAttemptStatus::Failed
        },
        scores,
        model: config.model.clone(),
        error: if complete {
            String::new()
        } else {
            "Decisions response omitted a configured noul answer".into()
        },
    })
}

pub async fn classify(
    reader: &impl SignalReader,
    completion: &impl Decisions,
    scope: &Scope,
    execution: &Execution,
    config: &SignalConfig,
) -> SignalAttempt {
    match attempt(reader, completion, scope, execution, config).await {
        Ok(attempt) => attempt,
        Err(error) => SignalAttempt {
            status: SignalAttemptStatus::Failed,
            scores: BTreeMap::new(),
            model: config.model.clone(),
            error: error.to_string().chars().take(300).collect(),
        },
    }
}
