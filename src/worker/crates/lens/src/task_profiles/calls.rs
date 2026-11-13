use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
};

use lens_server::profiles::{
    ProfileStoreError,
    aggregate::{Call, Charge, Column, Run, RunCalls, run_key},
};
use litellm_storage_clickhouse::Connection;
use litellm_traces::{
    LedgerRequest, ModelCall, SpendLookup, model_calls,
    query::named::{
        ReadAccessParams, SpendByResponseIdsParams, SpendByResponseIdsRow, TracePageSpansParams,
        TraceSpansRow,
    },
};
use litellm_traces_cache::TraceStore;
use litellm_traces_clickhouse::{
    ClickHouseTraces,
    query::profiles::{CallCostModels, CallCostModelsParams, CallCostModelsRow},
};

const RUNS_PER_READ: usize = 32;
const SPEND_MARGIN_MS: i64 = 30 * 60_000;
const NANOS_PER_MS: i64 = 1_000_000;

fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or_default()
}

pub(super) trait CallSource: Send + Sync {
    fn run_spans(
        &self,
        params: &TracePageSpansParams,
    ) -> impl Future<Output = Result<Vec<TraceSpansRow>, ProfileStoreError>> + Send;
    fn spend(
        &self,
        params: &SpendByResponseIdsParams,
    ) -> impl Future<Output = Result<Vec<SpendByResponseIdsRow>, ProfileStoreError>> + Send;
}

impl CallSource for ClickHouseTraces {
    async fn run_spans(
        &self,
        params: &TracePageSpansParams,
    ) -> Result<Vec<TraceSpansRow>, ProfileStoreError> {
        TraceStore::run_spans(self, params, now_ms())
            .await
            .map_err(|_| ProfileStoreError::Unavailable)
    }

    async fn spend(
        &self,
        params: &SpendByResponseIdsParams,
    ) -> Result<Vec<SpendByResponseIdsRow>, ProfileStoreError> {
        TraceStore::spend(self, params)
            .await
            .map_err(|_| ProfileStoreError::Unavailable)
    }
}

/// Resolves model calls for runs in batches: one span read and one spend read per batch, never
/// one per run. Stops before `max_spans` is exceeded; runs left out are reported as unresolved.
pub(super) async fn resolve(
    traces: &impl CallSource,
    client: &litellm_http::Client,
    reader: &Connection,
    scope: &ReadAccessParams,
    runs: &[Run],
    max_spans: usize,
) -> Result<RunCalls, ProfileStoreError> {
    let mut resolved = RunCalls::new();
    let mut spans_read = 0;
    for batch in runs.chunks(RUNS_PER_READ) {
        let (Some(start_ms), Some(end_ms)) = (
            batch.iter().map(|run| run.start_ms).min(),
            batch.iter().map(|run| run.end_ms.max(run.start_ms)).max(),
        ) else {
            continue;
        };
        let params = TracePageSpansParams {
            access: scope.clone(),
            trace_refs: batch
                .iter()
                .map(lens_server::profiles::aggregate::trace_ref)
                .collect(),
            trace_ids: batch.iter().map(|run| run.trace_id.clone()).collect(),
            start_ms,
            end_ms: end_ms.saturating_add(1),
        };
        let mut spans = traces.run_spans(&params).await?;
        spans_read += spans.len();
        if spans_read > max_spans {
            break;
        }
        spans.sort_by(|left, right| {
            (
                &left.team_id,
                &left.api_key_hash,
                &left.trace_id,
                left.start_ns,
            )
                .cmp(&(
                    &right.team_id,
                    &right.api_key_hash,
                    &right.trace_id,
                    right.start_ns,
                ))
        });
        let spend = spend(traces, scope, &spans).await?;
        let models = ledger_models(client, reader, scope, &spend).await?;
        let by_run: HashMap<(String, String, String), &[TraceSpansRow]> = spans
            .chunk_by(|left, right| {
                (&left.team_id, &left.api_key_hash, &left.trace_id)
                    == (&right.team_id, &right.api_key_hash, &right.trace_id)
            })
            .map(|rows| {
                (
                    (
                        rows[0].team_id.clone(),
                        rows[0].api_key_hash.clone(),
                        rows[0].trace_id.clone(),
                    ),
                    rows,
                )
            })
            .collect();
        for run in batch {
            let key = run_key(run);
            let calls = by_run
                .get(&key)
                .map(|rows| model_calls(rows, &spend))
                .unwrap_or_default()
                .into_iter()
                .map(|call| priced(call, &models))
                .collect();
            resolved.insert(key, calls);
        }
    }
    Ok(resolved)
}

async fn spend(
    traces: &impl CallSource,
    scope: &ReadAccessParams,
    spans: &[TraceSpansRow],
) -> Result<Vec<SpendByResponseIdsRow>, ProfileStoreError> {
    let lookup = SpendLookup::new(spans);
    let (Some(start_ns), Some(end_ns)) = (
        spans.iter().map(|row| row.start_ns).min(),
        spans
            .iter()
            .map(|row| row.start_ns.saturating_add_unsigned(row.duration_ns))
            .max(),
    ) else {
        return Ok(Vec::new());
    };
    if lookup.is_empty() {
        return Ok(Vec::new());
    }
    traces
        .spend(&SpendByResponseIdsParams {
            access: scope.clone(),
            response_ids: lookup.response_ids,
            provider_request_ids: lookup.provider_request_ids,
            request_ids: lookup.request_ids,
            trace_ids: lookup.trace_ids,
            start_ms: start_ns.div_euclid(NANOS_PER_MS) - SPEND_MARGIN_MS,
            end_ms: end_ns.div_euclid(NANOS_PER_MS) + SPEND_MARGIN_MS,
        })
        .await
}

type Ledger = BTreeMap<LedgerRequest, CallCostModelsRow>;

async fn ledger_models(
    client: &litellm_http::Client,
    reader: &Connection,
    scope: &ReadAccessParams,
    spend: &[SpendByResponseIdsRow],
) -> Result<Ledger, ProfileStoreError> {
    let (Some(start_ms), Some(end_ms)) = (
        spend.iter().map(|row| row.start_ms).min(),
        spend.iter().map(|row| row.start_ms).max(),
    ) else {
        return Ok(Ledger::new());
    };
    let rows = litellm_storage_clickhouse::fetch::<CallCostModels>(
        client,
        reader,
        &CallCostModelsParams {
            all_teams: scope.all_teams,
            user_id: scope.user_id.clone(),
            team_ids: scope.team_ids.clone(),
            request_ids: spend.iter().map(|row| row.request_id.clone()).collect(),
            start_ms,
            end_ms: end_ms.saturating_add(1),
        },
    )
    .await
    .map_err(|_| ProfileStoreError::Unavailable)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                LedgerRequest {
                    team_id: row.team_id.clone(),
                    start_ms: row.start_ms,
                    request_id: row.request_id.clone(),
                },
                row,
            )
        })
        .collect())
}

fn ledger_column(identity: &LedgerRequest, ledger: &Ledger) -> Column {
    match ledger.get(identity) {
        Some(row) if !row.model_id.is_empty() => Column::Deployment {
            model_id: row.model_id.clone(),
            model_group: row.model_group.clone(),
            provider: row.provider.clone(),
        },
        Some(row) => Column::UnknownDeployment {
            model_group: [&row.model_group, &row.model]
                .into_iter()
                .find(|value| !value.is_empty())
                .cloned()
                .unwrap_or_else(|| "unknown".into()),
        },
        None => Column::UnknownDeployment {
            model_group: "unknown".into(),
        },
    }
}

fn priced(call: ModelCall, ledger: &Ledger) -> Call {
    Call {
        fallback: Column::Unmatched {
            model: if call.model.is_empty() {
                "unknown".into()
            } else {
                call.model.clone()
            },
        },
        charges: call
            .requests
            .into_iter()
            .map(|request| Charge {
                column: ledger_column(&request.identity, ledger),
                identity: request.identity,
                spend: request.spend,
            })
            .collect(),
        span_id: call.span_id,
        agent: call.agent,
        duration_ms: call.duration_ns as f64 / NANOS_PER_MS as f64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::sync::Mutex;

    struct FailsAfterFirstBatch {
        reads: Mutex<Vec<usize>>,
    }

    impl CallSource for FailsAfterFirstBatch {
        async fn run_spans(
            &self,
            params: &TracePageSpansParams,
        ) -> Result<Vec<TraceSpansRow>, ProfileStoreError> {
            let mut reads = self.reads.lock().expect("reads");
            reads.push(params.trace_ids.len());
            if reads.len() > 1 {
                return Err(ProfileStoreError::Unavailable);
            }
            Ok(Vec::new())
        }

        async fn spend(
            &self,
            _: &SpendByResponseIdsParams,
        ) -> Result<Vec<SpendByResponseIdsRow>, ProfileStoreError> {
            Ok(Vec::new())
        }
    }

    fn listed(index: usize) -> Run {
        Run {
            team_id: "team".into(),
            api_key_hash: "key".into(),
            trace_id: format!("trace-{index}"),
            start_ms: 1,
            end_ms: 2,
            profile_id: "unclassified".into(),
            category: String::new(),
            input_preview: String::new(),
            agent_count: 1,
        }
    }

    #[rstest]
    #[tokio::test]
    async fn a_failed_span_read_fails_resolution_instead_of_looking_like_truncation() {
        let source = FailsAfterFirstBatch {
            reads: Mutex::default(),
        };
        let runs: Vec<Run> = (0..=RUNS_PER_READ).map(listed).collect();

        let resolved = resolve(
            &source,
            &crate::config::http_client().expect("client"),
            &Connection::reader("http://127.0.0.1:9", "unused").expect("connection"),
            &ReadAccessParams {
                all_teams: true,
                user_id: String::new(),
                team_ids: Vec::new(),
            },
            &runs,
            usize::MAX,
        )
        .await;

        assert_eq!(resolved, Err(ProfileStoreError::Unavailable));
        assert_eq!(*source.reads.lock().expect("reads"), vec![RUNS_PER_READ, 1]);
    }

    fn request(request_id: &str) -> LedgerRequest {
        LedgerRequest {
            team_id: "team".into(),
            start_ms: 1,
            request_id: request_id.into(),
        }
    }

    fn row(request: &LedgerRequest, model_id: &str, model_group: &str) -> CallCostModelsRow {
        CallCostModelsRow {
            team_id: request.team_id.clone(),
            request_id: request.request_id.clone(),
            start_ms: request.start_ms,
            model_id: model_id.into(),
            model_group: model_group.into(),
            model: "provider/model".into(),
            provider: "anthropic".into(),
        }
    }

    #[rstest]
    fn a_ledger_row_with_a_deployment_id_names_its_column() {
        let only = request("only");
        let ledger: Ledger = [(only.clone(), row(&only, "dep-1", "sonnet"))]
            .into_iter()
            .collect();

        assert_eq!(ledger_column(&only, &ledger).key(), "id:dep-1");
    }

    #[rstest]
    fn a_historical_ledger_row_without_a_deployment_id_is_not_given_one() {
        let only = request("only");
        let ledger: Ledger = [(only.clone(), row(&only, "", "sonnet"))]
            .into_iter()
            .collect();

        assert_eq!(
            ledger_column(&only, &ledger),
            Column::UnknownDeployment {
                model_group: "sonnet".into()
            }
        );
    }

    #[rstest]
    fn an_unmatched_call_falls_back_to_its_span_model_only() {
        let call = ModelCall {
            span_id: "span".into(),
            agent: "agent".into(),
            model: "gpt-5".into(),
            duration_ns: 2_000_000,
            state: None,
            requests: Vec::new(),
            spend: None,
        };

        let priced = priced(call, &Ledger::new());

        assert!(priced.charges.is_empty());
        assert_eq!(priced.fallback.key(), "unmatched:gpt-5");
        assert_eq!(priced.duration_ms, 2.0);
    }
}
