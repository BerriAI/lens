use crate::{EvaluationError, State, storage::TraceApi};
use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::eval::TraceAttribute;
use lens_evals::{EvalSpan, SpanStatus, StoredRun, Submission, Trial, TrialOutcome};
use lens_server::tracing::TraceReader;
use litellm_storage_clickhouse::fetch;
use litellm_traces::{query::named::ReadAccessParams, request::TraceDetailRequest};
use litellm_traces_clickhouse::query::evals::{
    EvalTrace, EvalTraceAttribute, EvalTraceParams, EvalTraceRow,
};
use std::{collections::BTreeSet, sync::Arc};

pub(super) struct ResolvedTrial {
    pub trial: Trial,
    pub traces: Vec<lens_contract::feedback::TraceIdentity>,
}

pub(super) async fn trial(
    state: &Arc<State>,
    run: &StoredRun,
    submission: &Submission,
    now: DateTime<Utc>,
) -> Result<Option<ResolvedTrial>, EvaluationError> {
    let error = || ResolvedTrial {
        trial: Trial {
            outcome: TrialOutcome::Error,
            cost_usd: submission.result.cost_usd,
            trace_spend_usd: None,
        },
        traces: Vec::new(),
    };
    let Some(reference) = &submission.result.trace else {
        return Ok(Some(error()));
    };
    let remaining = run
        .spec
        .timeout_per_trial_ms
        .saturating_sub(submission.result.duration_ms.unwrap_or(0));
    let deadline = submission
        .received_at
        .checked_add_signed(TimeDelta::milliseconds(
            i64::try_from(remaining).map_err(|_| EvaluationError::InvalidTrace)?,
        ))
        .ok_or(EvaluationError::InvalidTrace)?;
    let rows = rows(
        state,
        EvalTraceParams {
            team: run.team_id.clone(),
            attribute: match reference.attribute {
                TraceAttribute::SessionId => EvalTraceAttribute::SessionId,
                TraceAttribute::TraceId => EvalTraceAttribute::TraceId,
            },
            value: reference.value.clone(),
            cursor: String::new(),
        },
    )
    .await?;
    if rows.is_empty() {
        return Ok((now >= deadline).then(error));
    }
    if rows
        .iter()
        .map(|row| row.api_key_hash.as_str())
        .collect::<BTreeSet<_>>()
        .len()
        != 1
    {
        return Err(EvaluationError::AmbiguousTrace);
    }
    let versions: BTreeSet<_> = rows
        .iter()
        .map(|row| row.version.as_str())
        .filter(|version| !version.is_empty())
        .collect();
    if versions.len() != 1 || !versions.contains(run.spec.version.as_str()) {
        return Ok(Some(error()));
    }
    let roots: Vec<_> = rows
        .iter()
        .filter(|row| row.parent_span_id.is_empty())
        .collect();
    let received = rows
        .iter()
        .map(|row| row.received_ms)
        .max()
        .unwrap_or_default();
    let closed = if roots.is_empty() {
        received
            .checked_add(120_000)
            .ok_or(EvaluationError::InvalidTrace)?
    } else {
        let ended = roots
            .iter()
            .map(|row| row.end_ns.div_euclid(1_000_000))
            .max()
            .unwrap_or_default();
        received.max(u64::try_from(ended).map_err(|_| EvaluationError::InvalidTrace)?)
    };
    let closed = DateTime::from_timestamp_millis(
        i64::try_from(closed).map_err(|_| EvaluationError::InvalidTrace)?,
    )
    .ok_or(EvaluationError::InvalidTrace)?;
    if closed > now || closed > deadline {
        return Ok((now >= deadline).then(error));
    }
    let references: BTreeSet<_> = rows
        .iter()
        .map(|row| (&row.trace_id, &row.trace_ref))
        .collect();
    if reference.attribute == TraceAttribute::TraceId && references.len() != 1 {
        return Err(EvaluationError::AmbiguousTrace);
    }
    let traces = references
        .iter()
        .map(|(id, reference)| lens_contract::feedback::TraceIdentity {
            trace_id: (*id).clone(),
            trace_ref: (*reference).clone(),
        })
        .collect();
    let spend = if submission.result.cost_usd.is_some() {
        None
    } else {
        cost(state, &run.team_id, references).await?
    };
    let spans = rows.into_iter().map(span).collect();
    Ok(Some(ResolvedTrial {
        traces,
        trial: Trial {
            outcome: TrialOutcome::Trace(spans),
            cost_usd: submission.result.cost_usd,
            trace_spend_usd: spend,
        },
    }))
}

async fn rows(
    state: &Arc<State>,
    mut params: EvalTraceParams,
) -> Result<Vec<EvalTraceRow>, EvaluationError> {
    state.require_storage()?;
    let _permit = crate::wait_for_read_slot(state.read_slots.clone().acquire_owned()).await?;
    let mut rows = Vec::new();
    loop {
        let page = fetch::<EvalTrace>(
            &state.storage.client,
            state.storage.config.storage().reader(),
            &params,
        )
        .await?;
        let done = page.len() < 1000;
        if let Some(last) = page.last() {
            params.cursor = last.key.clone();
        }
        rows.extend(page);
        if done {
            return Ok(rows);
        }
        if rows.len() >= 20_000 {
            return Err(EvaluationError::TraceLimit);
        }
    }
}

fn span(row: EvalTraceRow) -> EvalSpan {
    let tool_name = if !row.tool_name.is_empty() {
        Some(row.tool_name)
    } else {
        (row.kind == "tool").then(|| row.name.clone())
    };
    EvalSpan {
        span_id: format!("{}:{}", row.trace_ref, row.span_id),
        parent_span_id: if row.parent_span_id.is_empty() {
            String::new()
        } else {
            format!("{}:{}", row.trace_ref, row.parent_span_id)
        },
        name: row.name,
        start_ns: row.start_ns,
        tool_name,
        input: row.input,
        output: row.output,
        status: match row.status {
            litellm_traces::SpanStatus::Ok => SpanStatus::Ok,
            litellm_traces::SpanStatus::Error => SpanStatus::Error,
            litellm_traces::SpanStatus::Unset => SpanStatus::Unset,
        },
    }
}

async fn cost(
    state: &Arc<State>,
    team: &str,
    references: BTreeSet<(&String, &String)>,
) -> Result<Option<f64>, EvaluationError> {
    let reader = TraceApi(state.clone());
    let mut total = None;
    for (trace_id, trace_ref) in references {
        let trace = reader
            .trace(
                ReadAccessParams {
                    all_teams: false,
                    user_id: String::new(),
                    team_ids: vec![team.into()],
                },
                trace_id.clone(),
                TraceDetailRequest {
                    trace_ref: trace_ref.clone(),
                    cursor: None,
                    page_size: None,
                },
            )
            .await?;
        if let Some(spend) = trace.and_then(|trace| trace.summary.spend) {
            total = Some(total.unwrap_or(0.0) + spend);
        }
    }
    Ok(total)
}
