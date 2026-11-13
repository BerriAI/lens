use lens_contract::{
    datasets::{DatasetCase, DatasetRole},
    eval::{TraceAttribute, TraceRef},
    optimizations::*,
};
use lens_server::tracing::TraceReader;
use litellm_traces::{
    ObservationType,
    query::named::ReadAccessParams,
    request::{TraceDetailRequest, TraceSpanRequest},
};
use serde_json::json;

use super::{
    Optimizations,
    store::{Attempt, AttemptState, Record, digest},
};
use crate::{error::OptimizationError as Error, storage::TraceApi};

pub(super) struct Source {
    pub id: String,
    pub request_id: Option<String>,
    pub started_at: i64,
    pub trace_id: String,
    pub trace_ref: String,
    pub span_id: String,
    pub input: String,
    pub expected: String,
    pub original: String,
    pub context: String,
    pub request: Option<ReplayRequest>,
    pub cost: Option<f64>,
    pub duration: Option<u64>,
    pub served_model: Option<String>,
    pub error: Option<String>,
}

pub(super) fn scope(team: &str) -> ReadAccessParams {
    ReadAccessParams {
        all_teams: false,
        user_id: String::new(),
        team_ids: vec![team.into()],
    }
}

impl Optimizations {
    pub(super) async fn trace_source(
        &self,
        record: &Record,
        trace_id: &str,
        trace_ref: &str,
    ) -> Result<Option<Source>, Error> {
        let config = &record.config;
        let state = &self.0.state;
        let reader = litellm_traces_clickhouse::evals::EvalTraces::new(
            state.storage.client.clone(),
            state.storage.config.storage().reader().clone(),
        );
        let Some(evidence) = reader
            .read(
                &config.team_id,
                &TraceRef {
                    attribute: TraceAttribute::TraceId,
                    value: trace_id.into(),
                },
                chrono::Utc::now().timestamp_millis(),
            )
            .await
            .map_err(|_| Error::Evidence)?
        else {
            return Ok(None);
        };
        let now = chrono::Utc::now().timestamp_millis();
        if evidence.agent != config.agent
            || (evidence.environment == "lens-eval" && config.scope != OptimizationScope::Agent)
            || now - evidence.last_received_at_ms < 2000
            || (evidence.root_ended_at_ms.is_none() && now - evidence.last_received_at_ms < 120_000)
        {
            return Ok(None);
        }
        let reference = evidence
            .traces
            .iter()
            .find(|trace| trace.trace_id == trace_id)
            .ok_or(Error::Evidence)?;
        if !trace_ref.is_empty() && trace_ref != reference.trace_ref {
            return Err(Error::Evidence);
        }
        let trace_ref = &reference.trace_ref;
        let api = TraceApi(state.clone());
        let trace = api
            .trace(
                scope(&config.team_id),
                trace_id.into(),
                TraceDetailRequest {
                    trace_ref: trace_ref.to_string(),
                    cursor: None,
                    page_size: Some(500),
                },
            )
            .await
            .map_err(|_| Error::Evidence)?
            .ok_or(Error::Evidence)?;
        if trace.next_cursor.is_some() {
            return Err(Error::Invalid("This trace exceeds the replay span limit"));
        }
        let mut calls: Vec<_> = trace
            .spans
            .iter()
            .filter(|span| span.kind == ObservationType::Llm)
            .collect();
        calls.sort_by(|left, right| left.start_offset_ms.total_cmp(&right.start_offset_ms));
        let Some(last) = calls.last() else {
            return Ok(None);
        };
        if calls
            .iter()
            .take(calls.len() - 1)
            .any(|span| span.start_offset_ms + span.duration_ms > last.start_offset_ms)
        {
            return Err(Error::Invalid("Parallel final model calls are ambiguous"));
        }
        let detail = api
            .span(
                scope(&config.team_id),
                trace_id.into(),
                last.span_id.clone(),
                TraceSpanRequest {
                    trace_ref: trace_ref.to_string(),
                },
            )
            .await
            .map_err(|_| Error::Evidence)?
            .ok_or(Error::Evidence)?;
        let context = evidence
            .spans
            .iter()
            .filter(|span| {
                span.span_id != format!("{trace_ref}:{}", last.span_id)
                    && !span.parent_span_id.is_empty()
                    && span.end_ns
                        <= evidence
                            .spans
                            .iter()
                            .find(|item| item.span_id == format!("{trace_ref}:{}", last.span_id))
                            .map_or(i64::MAX, |item| item.start_ns)
            })
            .map(|span| json!({"name":span.name,"input":span.input,"output":span.output}))
            .collect::<Vec<_>>();
        let request = ReplayRequest::capture(
            detail
                .attributes
                .get("lens.replay.request")
                .map_or(detail.input.as_str(), String::as_str),
        );
        let error = request
            .is_none()
            .then(|| "The final request was not captured in a replayable format".into());
        let original = match detail.output_ui {
            litellm_traces::UiContent::Messages { messages } => messages
                .into_iter()
                .map(|message| message.content)
                .collect::<Vec<_>>()
                .join("\n"),
            litellm_traces::UiContent::Text { text } => text,
            _ => detail.output,
        };
        Ok(Some(Source {
            id: trace_id.into(),
            request_id: evidence
                .spans
                .iter()
                .find(|span| span.parent_span_id.is_empty())
                .and_then(|span| span.attributes.get("lens.optimization.request_id").cloned()),
            started_at: evidence
                .spans
                .iter()
                .map(|span| span.start_ns / 1_000_000)
                .min()
                .unwrap_or(0),
            trace_id: trace_id.into(),
            trace_ref: trace_ref.to_string(),
            span_id: last.span_id.clone(),
            input: detail.input,
            expected: String::new(),
            original,
            context: serde_json::to_string(&context)?,
            request,
            cost: if config.mode == OptimizationMode::Live
                || config.scope == OptimizationScope::Agent
            {
                trace.summary.spend
            } else {
                last.spend
            },
            duration: Some(
                if config.mode == OptimizationMode::Live || config.scope == OptimizationScope::Agent
                {
                    trace.summary.duration_ms as u64
                } else {
                    last.duration_ms as u64
                },
            ),
            served_model: last.model.clone(),
            error,
        }))
    }

    pub(super) async fn offline_source(
        &self,
        record: &Record,
        case: &DatasetCase,
    ) -> Result<Source, Error> {
        let captured_source = if case.source.trace_id.is_empty() {
            None
        } else {
            self.trace_source(record, &case.source.trace_id, &case.source.trace_ref)
                .await?
        };
        let request = case
            .request
            .as_deref()
            .cloned()
            .or_else(|| {
                captured_source
                    .as_ref()
                    .and_then(|source| source.request.clone())
            })
            .or_else(|| {
                if !case.source.trace_id.is_empty()
                    || case.messages.iter().any(|message| {
                        message.role == DatasetRole::Tool || !message.tool_calls.is_empty()
                    })
                {
                    return None;
                }
                Some(ReplayRequest {
                    messages: case
                        .messages
                        .iter()
                        .map(|message| json!({"role":message.role,"content":message.content}))
                        .collect(),
                    parameters: Default::default(),
                })
            });
        let error = request
            .as_ref()
            .map_or(
                Some("The saved case has no replayable request"),
                |request| request.validate().err(),
            )
            .map(String::from);
        Ok(Source {
            id: case.id.clone(),
            request_id: None,
            started_at: 0,
            trace_id: case.source.trace_id.clone(),
            trace_ref: case.source.trace_ref.clone(),
            span_id: case.source.span_id.clone(),
            input: case
                .messages
                .iter()
                .find(|message| message.role == DatasetRole::User)
                .map(|message| message.content.clone())
                .unwrap_or_default(),
            expected: case.expected.clone(),
            original: case.reply.clone(),
            context: captured_source
                .as_ref()
                .map(|source| source.context.clone())
                .unwrap_or_default(),
            request,
            cost: captured_source.as_ref().and_then(|source| source.cost),
            duration: captured_source.as_ref().and_then(|source| source.duration),
            served_model: captured_source.and_then(|source| source.served_model),
            error,
        })
    }
}

impl Source {
    pub fn attempts(&self, record: &Record) -> Vec<Attempt> {
        std::iter::once(&record.config.baseline)
            .chain(record.config.models.iter())
            .map(|model| self.attempt(record, model))
            .collect()
    }

    pub fn attempt(&self, record: &Record, model: &str) -> Attempt {
        let full_agent = record.config.mode == OptimizationMode::Offline
            && record.config.scope == OptimizationScope::Agent;
        let recorded = (model == record.config.baseline
            && record.config.mode == OptimizationMode::Shadow)
            || record.config.mode == OptimizationMode::Live;
        let error = if recorded || full_agent {
            None
        } else {
            self.error.clone()
        };
        Attempt {
            result: OptimizationResult {
                id: digest(&[&self.id, model]),
                model: model.into(),
                served_model: recorded.then(|| self.served_model.clone()).flatten(),
                source_id: self.id.clone(),
                trace_id: self.trace_id.clone(),
                span_id: self.span_id.clone(),
                input: self.input.clone(),
                expected: self.expected.clone(),
                original: self.original.clone(),
                output: recorded.then(|| self.original.clone()),
                score: None,
                cost_usd: recorded.then_some(self.cost).flatten(),
                duration_ms: recorded.then_some(self.duration).flatten(),
                error: error.clone(),
                score_error: None,
                completed_at: if error.is_some() {
                    chrono::Utc::now().timestamp_millis()
                } else {
                    0
                },
            },
            request: if recorded || full_agent {
                None
            } else {
                self.request.clone()
            },
            context: self.context.clone(),
            trace_ref: self.trace_ref.clone(),
            state: if error.is_some() {
                AttemptState::Complete
            } else {
                AttemptState::Pending
            },
            owner: None,
            deadline: 0,
            external: full_agent,
        }
    }
}
