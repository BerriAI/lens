use crate::{
    Error,
    activity::Tracker,
    evidence::{Citation, Workspace, character_range},
    journal::{Journal, Turn as JournalTurn},
    model, sandbox, wire,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum Tool {
    Evidence(wire::EvidenceRequest),
    Python(wire::PythonRequest),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "T: DeserializeOwned"))]
struct Turn<T> {
    #[serde(default)]
    tools: Vec<Tool>,
    checkpoint: Option<String>,
    result: Option<T>,
}

pub fn checks(claim: &wire::Claim) -> Result<Vec<wire::Check>, Error> {
    let mut checks: Vec<_> = claim
        .job
        .settings
        .checks
        .iter()
        .filter(|check| check.enabled)
        .cloned()
        .collect();
    if !claim.job.settings.context.trim().is_empty() {
        checks.insert(0, serde_json::from_value(json!({"id": "expected_behavior", "instruction": "Identify deviations from the expected behavior described in context."}))?);
    }
    Ok(checks)
}

const REPAIR_ROUNDS: usize = 2;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum Target {
    Unread,
    Item(usize),
    Quote(usize, usize),
}

pub struct Rejection {
    target: Target,
    problem: String,
    cited: Option<(usize, wire::Evidence)>,
    closest: Option<String>,
}

impl Rejection {
    fn item(index: usize, problem: &str) -> Self {
        Self {
            target: Target::Item(index),
            problem: problem.into(),
            cited: None,
            closest: None,
        }
    }

    fn report(&self, items: &str) -> Value {
        let (path, cited) = match (self.target, &self.cited) {
            (Target::Unread, _) => ("result".into(), Value::Null),
            (Target::Item(item) | Target::Quote(item, _), Some((position, cited))) => (
                format!("result.{items}[{item}].evidence[{position}]"),
                json!({"execution_id": cited.execution_id, "span_id": cited.span_id, "quote": character_range(&cited.quote, 0, Some(400))}),
            ),
            (Target::Item(item) | Target::Quote(item, _), None) => {
                (format!("result.{items}[{item}]"), Value::Null)
            }
        };
        json!({"path": path, "problem": self.problem, "rejected_citation": cited, "recorded_text_near_quote": self.closest})
    }
}

pub trait Output: DeserializeOwned + Send + Sync {
    const SCHEMA: &'static str;
    const ITEMS: &'static str;
    fn verify(
        &mut self,
        claim: &wire::Claim,
        workspace: &Workspace,
    ) -> impl std::future::Future<Output = Result<Vec<Rejection>, Error>> + Send;
    fn discard(&mut self, rejections: &[Rejection]) -> bool;
    fn unsupported_without_reading(&self) -> bool {
        false
    }
}

async fn citations(
    workspace: &Workspace,
    enabled: &BTreeSet<String>,
    index: usize,
    check_id: &str,
    quotes: &mut Vec<wire::Evidence>,
) -> Result<Vec<Rejection>, Error> {
    if !enabled.contains(check_id) {
        return Ok(vec![Rejection::item(index, "Use an enabled check ID")]);
    }
    let mut rejections = Vec::new();
    let mut resolved = Vec::with_capacity(quotes.len());
    for (position, quote) in quotes.drain(..).enumerate() {
        let rejected = match workspace.cite(&quote).await {
            Ok(Citation::Verified(verified)) => {
                resolved.extend(verified);
                continue;
            }
            Ok(Citation::Rejected { problem, closest }) => (problem.to_owned(), closest),
            Err(error) => (
                format!(
                    "Could not verify this citation: {error}. Inspect other evidence and revise the citation."
                ),
                None,
            ),
        };
        rejections.push(Rejection {
            target: Target::Quote(index, resolved.len()),
            cited: Some((position, quote.clone())),
            problem: rejected.0,
            closest: rejected.1,
        });
        resolved.push(quote);
    }
    *quotes = resolved;
    if rejections.is_empty() && !quotes.iter().any(|q| q.role == wire::EvidenceRole::Support) {
        rejections.push(Rejection::item(index, "Each finding or observation needs at least one supporting quote from original evidence"));
    }
    Ok(rejections)
}

fn discard<T>(
    items: &mut Vec<T>,
    rejections: &[Rejection],
    evidence: impl Fn(&mut T) -> &mut Vec<wire::Evidence>,
) -> bool {
    let rejected: BTreeSet<_> = rejections
        .iter()
        .map(|rejection| rejection.target)
        .collect();
    let mut index = 0;
    items.retain_mut(|item| {
        let current = index;
        index += 1;
        let mut position = 0;
        evidence(item).retain(|_| {
            position += 1;
            !rejected.contains(&Target::Quote(current, position - 1))
        });
        !rejected.contains(&Target::Item(current))
            && evidence(item)
                .iter()
                .any(|quote| quote.role == wire::EvidenceRole::Support)
    });
    !items.is_empty()
}

fn enabled(claim: &wire::Claim) -> Result<BTreeSet<String>, Error> {
    Ok(checks(claim)?
        .into_iter()
        .map(|check| check.id.to_string())
        .collect())
}

impl Output for wire::Extraction {
    const SCHEMA: &'static str = "PythonAgentTurn[Extraction]";
    const ITEMS: &'static str = "observations";
    async fn verify(
        &mut self,
        claim: &wire::Claim,
        workspace: &Workspace,
    ) -> Result<Vec<Rejection>, Error> {
        let enabled = enabled(claim)?;
        let mut rejections = Vec::new();
        for (index, observation) in self.observations.iter_mut().enumerate() {
            rejections.extend(
                citations(
                    workspace,
                    &enabled,
                    index,
                    &observation.check_id,
                    &mut observation.evidence,
                )
                .await?,
            );
        }
        Ok(rejections)
    }

    fn discard(&mut self, rejections: &[Rejection]) -> bool {
        discard(&mut self.observations, rejections, |observation| {
            &mut observation.evidence
        })
    }

    fn unsupported_without_reading(&self) -> bool {
        self.cannot_assess
    }
}

impl Output for wire::Findings {
    const SCHEMA: &'static str = "PythonAgentTurn[Findings]";
    const ITEMS: &'static str = "findings";
    async fn verify(
        &mut self,
        claim: &wire::Claim,
        workspace: &Workspace,
    ) -> Result<Vec<Rejection>, Error> {
        let enabled = enabled(claim)?;
        let mut rejections = Vec::new();
        for (index, finding) in self.findings.iter_mut().enumerate() {
            finding.check_ids.retain(|id| enabled.contains(id));
            finding.merged_finding_ids.clear();
            if finding.existing_finding_id.as_ref().is_some_and(|id| {
                !claim
                    .findings
                    .iter()
                    .any(|f| &f.id == id && f.kind.to_string() == finding.kind.to_string())
            }) {
                finding.existing_finding_id = None;
            }
            rejections.extend(
                citations(
                    workspace,
                    &enabled,
                    index,
                    &finding.check_id,
                    &mut finding.evidence,
                )
                .await?,
            );
            if finding.kind == wire::FindingDraftKind::Issue && finding.brief.is_none() {
                rejections.push(Rejection::item(index, "Issues require a brief containing the problem, user goal, observed outcome, and test cases"));
            }
        }
        Ok(rejections)
    }

    fn discard(&mut self, rejections: &[Rejection]) -> bool {
        discard(&mut self.findings, rejections, |finding| {
            &mut finding.evidence
        })
    }
}

pub struct Assignment<'a> {
    pub stage: &'a str,
    pub task: String,
    pub purpose: wire::ModelRequestPurpose,
    pub supplied: Value,
}

pub async fn run<T: Output>(
    claim: &wire::Claim,
    workspace: &Workspace,
    assignment: Assignment<'_>,
    tracker: &Tracker,
) -> Result<T, Error> {
    let existing: Vec<Value> = claim
        .findings
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|mut finding| {
            if let Some(object) = finding.as_object_mut() {
                for field in ["evidence", "occurrences", "investigation_runs"] {
                    object.remove(field);
                }
            }
            finding
        })
        .collect();
    let initial =
        json!({"evidence": [], "supplied": assignment.supplied, "existing_findings": existing});
    let mut journal = Journal::new(&initial).await?;
    let prompt = json!({
        "stage": assignment.stage, "task": assignment.task,
        "response_instructions": include_str!("../prompts/response_instructions.md"),
        "tool_instructions": include_str!("../prompts/tool_instructions.md"),
        "python_instructions": include_str!("../prompts/python_instructions.md"),
        "context": claim.job.settings.context, "checks": checks(claim)?,
        "catalog_fields": ["span_id", "parent_span_id", "name", "kind", "characters", "start_time", "end_time"],
        "execution_ids": workspace.executions.iter().map(|execution| &execution.id).collect::<Vec<_>>(),
        "available_sessions": workspace.executions.len(), "available_review_records": workspace.reviews.len(),
        "response_schema": model::schema(T::SCHEMA)?,
    });
    let mut request = model::request(assignment.purpose, prompt)?;
    let task_message = model::message(wire::ModelMessageRole::System, request.prompt.to_string());
    request.messages = vec![task_message.clone(), model::message(wire::ModelMessageRole::User, json!({"initial_evidence": [], "supplied": assignment.supplied, "existing_findings": existing}).to_string())];
    let mut compacted = false;
    let mut rejected = 0;
    let mut inspected = false;
    let mut history_characters = 16 * 1024;
    let mut repeated_tools = BTreeMap::<String, u8>::new();
    loop {
        tracker.change("model", true).await?;
        let result = model::structured::<Turn<T>>(&workspace.client, request.clone(), T::SCHEMA, |turn| {
            if (turn.tools.is_empty() && turn.checkpoint.is_none()) != turn.result.is_some() {
                return Some("Return tools and/or a checkpoint with result=null, or a final result without tools or checkpoint".into());
            }
            if turn.checkpoint.as_ref().is_some_and(|c| c.is_empty()) { return Some("Checkpoint must not be empty".into()); }
            None
        }).await;
        tracker.change("model", false).await?;
        let (turn, responded) = match result {
            Err(Error::Context(previous)) if !compacted => {
                history_characters = (history_characters / 2).max(1);
                tracker.change("checkpoint", true).await?;
                request.messages =
                    model::compact(&workspace.client, *previous, journal.turns.len() + 1).await?;
                tracker.change("checkpoint", false).await?;
                journal
                    .push(&JournalTurn {
                        response: request.messages[1].content.clone(),
                        tool_results: Vec::new(),
                        validation_error: String::new(),
                    })
                    .await?;
                compacted = true;
                continue;
            }
            Err(Error::Context(_)) => {
                return Err(Error::CompactedContext);
            }
            result => result?,
        };
        compacted = false;
        if let Some(mut result) = turn.result {
            let rejections = if !inspected && result.unsupported_without_reading() {
                vec![Rejection {
                    target: Target::Unread,
                    problem: "You have not read the recorded spans. Use catalog and read on the assigned execution before deciding that it cannot be assessed.".into(),
                    cited: None,
                    closest: None,
                }]
            } else {
                result.verify(claim, workspace).await?
            };
            if rejections.is_empty() {
                return Ok(result);
            }
            rejected += 1;
            let reports: Vec<Value> = rejections
                .iter()
                .map(|rejection| rejection.report(T::ITEMS))
                .collect();
            let invalid = format!(
                "{} of the cited results could not be verified. First problem: {}",
                rejections.len(),
                rejections[0].problem
            );
            journal
                .push(&JournalTurn {
                    response: responded
                        .last()
                        .ok_or(Error::InvalidRequest)?
                        .content
                        .clone(),
                    tool_results: Vec::new(),
                    validation_error: invalid.clone(),
                })
                .await?;
            if rejected > REPAIR_ROUNDS {
                if rejections[0].target != Target::Unread && result.discard(&rejections) {
                    tracing::warn!(
                        schema = T::SCHEMA,
                        discarded = rejections.len(),
                        "Lens kept the verified results and discarded the rest"
                    );
                    return Ok(result);
                }
                return Err(Error::ModelValidation {
                    schema: T::SCHEMA,
                    attempts: rejected,
                    detail: invalid,
                });
            }
            request.messages = responded;
            request.messages.push(model::message(
                wire::ModelMessageRole::User,
                json!({"journal_turns": journal.turns.len()}).to_string(),
            ));
            request.messages.push(model::message(wire::ModelMessageRole::System, json!({"instruction": "Some parts of your result were rejected. Return the complete result again with each rejected part corrected or removed. For a rejected citation, copy the supporting text character for character from recorded_text_near_quote or from a tool result, keep execution_id and span_id of the span that contains it, and remove the claim if the recording does not support it. Tools remain available. Continue using the task response_schema.", "rejected": reports}).to_string()));
            continue;
        }
        let mut results = Vec::new();
        let mut archived = Vec::new();
        for tool in turn.tools {
            inspected = true;
            let arguments = serde_json::to_string(&tool)?;
            let repeat_key = match &tool {
                Tool::Evidence(request)
                    if request.action == wire::EvidenceRequestAction::History =>
                {
                    format!("{history_characters}:{arguments}")
                }
                _ => arguments,
            };
            let repeated = repeated_tools.entry(repeat_key).or_default();
            *repeated += 1;
            if *repeated > 3 {
                return Err(Error::ToolNoProgress);
            }
            let operation = match &tool {
                Tool::Evidence(r) => r.action.to_string(),
                Tool::Python(_) => "python".into(),
            };
            tracker.change(&operation, true).await?;
            let result = match &tool {
                Tool::Evidence(request)
                    if request.action == wire::EvidenceRequestAction::History =>
                {
                    journal.page(request, history_characters).await
                }
                Tool::Evidence(request) => workspace.respond(request).await,
                Tool::Python(request) => sandbox::execute(workspace, request)
                    .await
                    .map(|output| json!({"request": request, "output": output})),
            };
            tracker.change(&operation, false).await?;
            let result = match result {
                Ok(value) => value,
                Err(error) => json!({"request": tool, "error": error.to_string()}),
            };
            archived.push(match &tool {
                Tool::Evidence(r) if r.action == wire::EvidenceRequestAction::History => result
                    .get("request")
                    .and_then(|value| serde_json::from_value(value.clone()).ok())
                    .as_ref()
                    .and_then(|request| journal.reference(request))
                    .unwrap_or_else(|| result.to_string()),
                _ => result.to_string(),
            });
            results.push(result);
        }
        journal
            .push(&JournalTurn {
                response: responded
                    .last()
                    .ok_or(Error::InvalidRequest)?
                    .content
                    .clone(),
                tool_results: archived,
                validation_error: String::new(),
            })
            .await?;
        request.messages = if let Some(checkpoint) = turn.checkpoint {
            tracker.change("checkpoint", true).await?;
            let messages = vec![
                task_message.clone(),
                model::message(
                    wire::ModelMessageRole::User,
                    json!({"working_notes": checkpoint, "initial_context_archived": true})
                        .to_string(),
                ),
                responded.last().ok_or(Error::InvalidRequest)?.clone(),
            ];
            tracker.change("checkpoint", false).await?;
            messages
        } else {
            responded
        };
        request.messages.push(model::message(
            wire::ModelMessageRole::User,
            json!({"journal_turns": journal.turns.len(), "tool_results": results}).to_string(),
        ));
        if request
            .messages
            .iter()
            .map(|m| m.content.len())
            .sum::<usize>()
            > 16 * 1024 * 1024
        {
            request.messages =
                model::compact(&workspace.client, request.clone(), journal.turns.len()).await?;
            compacted = true;
        }
    }
}
