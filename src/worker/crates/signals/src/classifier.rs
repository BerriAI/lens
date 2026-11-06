use std::{
    collections::BTreeMap,
    future::Future,
    ops::Range,
    time::{Duration, Instant},
};

use lens_contract::{
    investigations::Scope,
    signals::{
        NoulAnswer, SignalAttempt, SignalAttemptStatus, SignalConfig, SignalEvidence,
        SignalFailure, SignalStep,
    },
    worker::Execution,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{DecisionsError, Error, SignalReader};

const PASSAGE_CHARACTERS: usize = 600;
// Choice supports 255 options including none: https://docs.typesafe.ai/primitives/choice (2026-11-06).
const EVIDENCE_OPTIONS: usize = 254;

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

struct SourceStep {
    step: SignalStep,
    span_id: String,
}

struct EvidenceState {
    state: SignalState,
    candidates: BTreeMap<String, SignalEvidence>,
}

async fn source_steps(
    reader: &impl SignalReader,
    scope: &Scope,
    execution: &Execution,
) -> Result<Vec<SourceStep>, Error> {
    let mut steps = Vec::new();
    let mut cursor = String::new();
    loop {
        let content = reader.content(scope, execution, &cursor).await?;
        steps.extend(content.parts.into_iter().map(|part| SourceStep {
            step: SignalStep {
                kind: part.kind,
                name: part.name,
                content: part.content,
            },
            span_id: part.span_id,
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
        source_steps(reader, scope, execution)
            .await?
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
    for source in steps {
        let content = passages(&source.step.content)
            .into_iter()
            .map(|passage| {
                let quote = passage.trim();
                if !source.span_id.is_empty() && quotable(quote) {
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

fn selected_choice(value: &Value) -> Option<String> {
    let EvidenceAnswer::Choice { choice, confidence } =
        serde_json::from_value(value.clone()).ok()?;
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return None;
    }
    Some(choice)
}

fn evidence_choices(range: Range<usize>) -> BTreeMap<String, Range<usize>> {
    let width = range.len().div_ceil(EVIDENCE_OPTIONS).max(1);
    range
        .clone()
        .step_by(width)
        .map(|start| {
            let end = start.saturating_add(width).min(range.end);
            let name = if end == start + 1 {
                format!("L{start:03}")
            } else {
                format!("L{start:03}..L{:03}", end - 1)
            };
            (name, start..end)
        })
        .collect()
}

fn evidence_question(question: &str, choices: &BTreeMap<String, Range<usize>>) -> Question {
    let criteria = choices
        .iter()
        .map(|(name, range)| {
            (
                name.clone(),
                (range.len() > 1).then(|| {
                    format!(
                        "All excerpts from [L{:03}] through [L{:03}], inclusive",
                        range.start,
                        range.end - 1
                    )
                }),
            )
        })
        .chain([(
            "none".into(),
            Some("No excerpt directly supports a yes answer".into()),
        )])
        .collect();
    Question::Choice {
        instructions: format!(
            "Which labeled excerpt or range contains the clearest direct evidence for a yes answer to this question? {question} Select none if no excerpt directly supports a yes answer. Judge the recorded text, not the labels or omission notices."
        ),
        criteria,
    }
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

fn output(value: Value) -> Result<Output, Error> {
    let object: serde_json::Map<String, Value> =
        serde_json::from_value(value).map_err(Error::Response)?;
    serde_json::from_value(Value::Object(object)).map_err(Error::Response)
}

async fn attempt(
    reader: &impl SignalReader,
    completion: &impl Decisions,
    scope: &Scope,
    execution: &Execution,
    config: &SignalConfig,
) -> Result<SignalAttempt, Error> {
    let prepared = evidence_state(source_steps(reader, scope, execution).await?);
    let choices = evidence_choices(0..prepared.candidates.len());
    let mut pending = config
        .signals
        .iter()
        .enumerate()
        .filter(|_| !choices.is_empty())
        .map(|(index, _)| (index, choices.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut request = DecisionRequest {
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
                let evidence = pending.get(&index).map(|choices| {
                    (
                        evidence_question_id(index),
                        evidence_question(&signal.question, choices),
                    )
                });
                std::iter::once(score).chain(evidence)
            })
            .collect(),
        timeout: Duration::from_secs(60),
        tags: vec!["litellm-lens-signals"],
    };
    let started = Instant::now();
    let mut response = output(completion.complete(&request).await?)?;
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
    let mut evidence = BTreeMap::new();
    let mut evidence_error = None;
    while !pending.is_empty() {
        let mut next = BTreeMap::new();
        for (index, choices) in pending {
            let Some(range) = response
                .answers
                .get(&evidence_question_id(index))
                .and_then(selected_choice)
                .and_then(|choice| choices.get(&choice).cloned())
            else {
                continue;
            };
            if range.len() == 1 {
                if let Some(selected) = prepared.candidates.get(&format!("L{:03}", range.start)) {
                    evidence.insert(config.signals[index].id.clone(), selected.clone());
                }
            } else {
                next.insert(index, evidence_choices(range));
            }
        }
        pending = next;
        if pending.is_empty() || !complete {
            break;
        }
        request.questions = pending
            .iter()
            .map(|(index, choices)| {
                (
                    evidence_question_id(*index),
                    evidence_question(&config.signals[*index].question, choices),
                )
            })
            .collect();
        let followup = async {
            request.timeout = Duration::from_secs(60)
                .checked_sub(started.elapsed())
                .filter(|remaining| !remaining.is_zero())
                .ok_or(DecisionsError::Timeout)?;
            output(completion.complete(&request).await?)
        }
        .await;
        match followup {
            Ok(next) => response = next,
            Err(error) => {
                tracing::warn!(reason = %error, "Lens signal evidence lookup failed");
                evidence_error = Some(error);
                break;
            }
        }
    }
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
        failure: evidence_error
            .as_ref()
            .and_then(provider_failure)
            .filter(|failure| matches!(failure, SignalFailure::Transient { .. })),
    })
}

fn provider_failure(error: &Error) -> Option<SignalFailure> {
    match error {
        Error::Decisions(DecisionsError::Provider {
            status: 400 | 413 | 422,
            ..
        }) => Some(SignalFailure::Permanent),
        Error::Decisions(DecisionsError::Provider {
            retry_after_seconds,
            ..
        }) => Some(SignalFailure::Transient {
            retry_after_seconds: *retry_after_seconds,
        }),
        Error::Decisions(DecisionsError::Timeout | DecisionsError::Unavailable(_)) => {
            Some(SignalFailure::Transient {
                retry_after_seconds: None,
            })
        }
        _ => None,
    }
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
            error: error.to_string(),
            failure: provider_failure(&error),
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
                content: content.into(),
            },
            span_id: "span".into(),
        }]);
        assert!(prepared.candidates.is_empty());
    }

    #[rstest]
    #[case::unicode_and_source_omission(120, format!("Input: {}\n[... content omitted ...]\nOutput: {}\nStatus: OK", "🗿".repeat(5000), "雪".repeat(2000)))]
    #[case::many_lines(120, "Useful evidence\n".repeat(100))]
    #[case::many_candidates(400, "Useful evidence".into())]
    fn selectable_passages_are_visible_literal_source_text(
        #[case] count: usize,
        #[case] content: String,
    ) {
        let prepared = evidence_state(
            (0..count)
                .map(|index| SourceStep {
                    step: SignalStep {
                        kind: "user".into(),
                        name: "message".into(),
                        content: content.clone(),
                    },
                    span_id: format!("span-{index}"),
                })
                .collect(),
        );
        assert!(!prepared.candidates.is_empty());
        assert_eq!(prepared.state.steps.len(), count);
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
