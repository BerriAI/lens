use std::{collections::BTreeMap, future::Future, time::Duration};

use lens_contract::{
    investigations::Scope,
    signals::{
        NoulAnswer, SignalAttempt, SignalAttemptStatus, SignalConfig, SignalEvidence, SignalStep,
    },
    worker::Execution,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{DecisionsError, Error, SignalReader};

const MAX_CANDIDATES: usize = 254;
const STATE_CHARACTERS: usize = 40_000;
const PASSAGE_CHARACTERS: usize = 600;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SignalState {
    pub task: &'static str,
    pub steps: Vec<SignalStep>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Question {
    Noul {
        instructions: String,
    },
    Choice {
        instructions: String,
        criteria: BTreeMap<String, Option<String>>,
    },
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

#[derive(Clone)]
struct SourceStep {
    step: SignalStep,
    span_id: String,
    source: String,
}

struct EvidenceState {
    state: SignalState,
    candidates: BTreeMap<String, SignalEvidence>,
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
    steps: impl Iterator<Item = SourceStep>,
    mut remaining: usize,
    tail: bool,
) -> Vec<SourceStep> {
    let mut result = Vec::new();
    for step in steps {
        if remaining == 0 {
            break;
        }
        let length = step.step.content.chars().count();
        let used = remaining.min(length);
        let content = if tail {
            step.step.content.chars().skip(length - used).collect()
        } else {
            step.step.content.chars().take(used).collect()
        };
        result.push(SourceStep {
            step: SignalStep {
                content,
                ..step.step
            },
            ..step
        });
        remaining -= used;
    }
    result
}

fn bounded(steps: Vec<SourceStep>, limit: usize) -> Vec<SourceStep> {
    if steps
        .iter()
        .map(|step| step.step.content.chars().count())
        .sum::<usize>()
        <= limit
    {
        return steps;
    }
    let head_budget = limit * 3 / 8;
    let head = take(steps.iter().cloned(), head_budget, false);
    let mut tail = take(steps.iter().rev().cloned(), limit - head_budget, true);
    tail.reverse();
    let omitted = steps.len() as i64 - head.len() as i64 - tail.len() as i64;
    head.into_iter()
        .chain([SourceStep {
            step: SignalStep {
                kind: "omitted".into(),
                name: String::new(),
                content: format!("{omitted} steps omitted"),
            },
            span_id: String::new(),
            source: String::new(),
        }])
        .chain(tail)
        .collect()
}

async fn source_steps(
    reader: &impl SignalReader,
    scope: &Scope,
    execution: &Execution,
) -> Result<Vec<SourceStep>, Error> {
    let mut steps = Vec::new();
    let mut cursor = String::new();
    for _ in 0..3 {
        let content = reader.content(scope, execution, &cursor).await?;
        steps.extend(content.parts.into_iter().map(|part| SourceStep {
            step: SignalStep {
                kind: part.kind,
                name: part.name,
                content: excerpt(&part.content),
            },
            span_id: part.span_id,
            source: part.content,
        }));
        let Some(next) = content.next_cursor else {
            break;
        };
        cursor = next;
    }
    Ok(steps)
}

fn state(steps: Vec<SignalStep>) -> SignalState {
    SignalState {
        task: "An AI agent run recorded as a trace. Judge only what the user and the agent said and did in these steps.",
        steps,
    }
}

pub async fn signal_state(
    reader: &impl SignalReader,
    scope: &Scope,
    execution: &Execution,
) -> Result<SignalState, Error> {
    Ok(state(
        bounded(
            source_steps(reader, scope, execution).await?,
            STATE_CHARACTERS,
        )
        .into_iter()
        .map(|source| source.step)
        .collect(),
    ))
}

fn passages(content: &str) -> Vec<String> {
    let mut passages = Vec::new();
    let mut current = String::new();
    let mut length = 0;
    for line in content.split_inclusive('\n') {
        let characters = line.chars().count();
        let omitted = line.trim_start().starts_with("[...");
        if length + characters > PASSAGE_CHARACTERS || omitted {
            if !current.is_empty() {
                passages.push(std::mem::take(&mut current));
            }
            length = 0;
        }
        if characters > PASSAGE_CHARACTERS {
            passages.extend(
                line.chars()
                    .collect::<Vec<_>>()
                    .chunks(PASSAGE_CHARACTERS)
                    .map(|chunk| chunk.iter().collect()),
            );
        } else if omitted {
            passages.push(line.into());
        } else {
            current.push_str(line);
            length += characters;
        }
    }
    if !current.is_empty() {
        passages.push(current);
    }
    passages
}

fn quotable(quote: &str) -> bool {
    !quote.starts_with("[...")
        && quote.lines().any(|line| {
            !matches!(
                line.trim(),
                "" | "Input:" | "Output:" | "Status:" | "Status: OK" | "Error:"
            )
        })
}

fn evidence_state(steps: Vec<SourceStep>) -> EvidenceState {
    let mut candidates = BTreeMap::new();
    let mut annotated = Vec::new();
    for source in bounded(steps, STATE_CHARACTERS - MAX_CANDIDATES * 8 - 64) {
        let content = passages(&source.step.content)
            .into_iter()
            .map(|passage| {
                let quote = passage.trim();
                if candidates.len() < MAX_CANDIDATES
                    && !source.span_id.is_empty()
                    && quotable(quote)
                    && source.source.contains(quote)
                {
                    let id = format!("L{:03}", candidates.len());
                    candidates.insert(
                        id.clone(),
                        SignalEvidence {
                            span_id: source.span_id.clone(),
                            quote: quote.into(),
                        },
                    );
                    format!("[{id}] {quote}")
                } else {
                    quote.into()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        annotated.push(SignalStep {
            content,
            ..source.step
        });
    }
    EvidenceState {
        state: state(annotated),
        candidates,
    }
}

fn evidence_question_id(index: usize) -> String {
    format!("__evidence_{index}")
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum EvidenceAnswer {
    Choice { choice: String, confidence: f64 },
}

fn selected_evidence(
    value: &Value,
    candidates: &BTreeMap<String, SignalEvidence>,
) -> Option<SignalEvidence> {
    let EvidenceAnswer::Choice { choice, confidence } =
        serde_json::from_value(value.clone()).ok()?;
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return None;
    }
    candidates.get(&choice).cloned()
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
    let prepared = evidence_state(source_steps(reader, scope, execution).await?);
    let criteria = prepared
        .candidates
        .keys()
        .map(|id| (id.clone(), None))
        .chain([(
            "none".into(),
            Some("No excerpt directly supports a yes answer".into()),
        )])
        .collect::<BTreeMap<_, _>>();
    let request = DecisionRequest {
        model: config.model.clone(),
        state: prepared.state,
        questions: config
            .signals
            .iter()
            .enumerate()
            .flat_map(|(index, signal)| {
                let score = (
                    signal.id.clone(),
                    Question::Noul {
                        instructions: signal.question.clone(),
                    },
                );
                let evidence = (!prepared.candidates.is_empty()).then(|| (
                    evidence_question_id(index),
                    Question::Choice {
                        instructions: format!("Which labeled excerpt provides the clearest direct evidence for a yes answer to this question? {} Select none if no excerpt directly supports a yes answer. Judge the recorded text, not the labels or omission notices.", signal.question),
                        criteria: criteria.clone(),
                    },
                ));
                std::iter::once(score).chain(evidence)
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
    let evidence = config
        .signals
        .iter()
        .enumerate()
        .filter_map(|(index, signal)| {
            response
                .answers
                .get(&evidence_question_id(index))
                .and_then(|answer| selected_evidence(answer, &prepared.candidates))
                .map(|evidence| (signal.id.clone(), evidence))
        })
        .collect();
    Ok(SignalAttempt {
        status: if complete {
            SignalAttemptStatus::Classified
        } else {
            SignalAttemptStatus::Failed
        },
        scores,
        evidence,
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
            evidence: BTreeMap::new(),
            model: config.model.clone(),
            error: error.to_string().chars().take(300).collect(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::empty("")]
    #[case::headers("Input:\nOutput:\nStatus: OK\nError:")]
    #[case::source_omission("[... content omitted ...]")]
    fn unavailable_source_content_never_becomes_an_evidence_choice(#[case] content: &str) {
        let prepared = evidence_state(vec![SourceStep {
            step: SignalStep {
                kind: "llm".into(),
                name: "model".into(),
                content: excerpt(content),
            },
            span_id: "span".into(),
            source: content.into(),
        }]);
        assert!(prepared.candidates.is_empty());
    }

    #[rstest]
    #[case::unicode_and_excerpt(120, format!("Input: {}\n[... content omitted ...]\nOutput: {}\nStatus: OK", "🗿".repeat(5000), "雪".repeat(2000)))]
    #[case::many_lines(120, "Useful evidence\n".repeat(100))]
    #[case::candidate_limit(400, "Useful evidence".into())]
    fn selectable_passages_are_bounded_visible_literal_source_text(
        #[case] count: usize,
        #[case] content: String,
    ) {
        let prepared = evidence_state(
            (0..count)
                .map(|index| SourceStep {
                    step: SignalStep {
                        kind: "user".into(),
                        name: "message".into(),
                        content: excerpt(&content),
                    },
                    span_id: format!("span-{index}"),
                    source: content.clone(),
                })
                .collect(),
        );
        assert!(!prepared.candidates.is_empty());
        assert!(prepared.candidates.len() <= MAX_CANDIDATES);
        assert!(
            prepared
                .state
                .steps
                .iter()
                .map(|step| step.content.chars().count())
                .sum::<usize>()
                <= STATE_CHARACTERS
        );
        let sent = prepared
            .state
            .steps
            .iter()
            .map(|step| step.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let invalid = prepared
            .candidates
            .iter()
            .filter(|(id, evidence)| {
                !content.contains(&evidence.quote)
                    || evidence.quote.contains("[...")
                    || evidence.quote.chars().count() > PASSAGE_CHARACTERS
                    || !sent.contains(&format!("[{id}] {}", evidence.quote))
                    || !evidence.span_id.starts_with("span-")
            })
            .collect::<Vec<_>>();
        assert!(invalid.is_empty(), "Invalid evidence: {invalid:?}");
    }
}
