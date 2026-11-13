use std::collections::BTreeMap;

use lens_server::profiles::{ProfileStore, RunQuery, aggregate::View};
use litellm_traces::{Tenant, decode_otlp, query::named::ReadAccessParams};
use litellm_traces_clickhouse::{InsertTable, insert_rows, insert_shared_rows, span_rows};
use rstest::{fixture, rstest};
use serde_json::{Value, json};

mod profile_store;
use profile_store::Isolated;

const AGENT: &str = "long-agent";
const TEAM: &str = "long-team";
const KEY: &str = "long-key";
const TRACE: &str = "0123456789abcdef0123456789abcdef";
const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;

fn every_team() -> ReadAccessParams {
    ReadAccessParams {
        all_teams: true,
        user_id: String::new(),
        team_ids: Vec::new(),
    }
}

fn attributes(pairs: &[(&str, &str)]) -> Value {
    Value::Array(
        pairs
            .iter()
            .map(|(key, value)| json!({"key": key, "value": {"stringValue": value}}))
            .collect(),
    )
}

fn span(span_id: &str, parent: &str, start_ms: i64, duration_ms: i64, pairs: &[(&str, &str)]) -> Value {
    json!({
        "traceId": TRACE, "spanId": span_id, "parentSpanId": parent, "name": pairs[0].1, "kind": 1,
        "startTimeUnixNano": (start_ms * 1_000_000).to_string(),
        "endTimeUnixNano": ((start_ms + duration_ms) * 1_000_000).to_string(),
        "attributes": attributes(pairs), "status": {"code": 1}
    })
}

fn ledger(request: &str, response: &str, start_ms: i64, spend: f64) -> BTreeMap<String, Value> {
    json!({
        "request_id": request, "response_id": response, "litellm_call_id": format!("call-{request}"),
        "team_id": TEAM, "api_key": KEY, "user": "long-user",
        "model": "anthropic/sonnet", "model_group": "sonnet", "model_id": "dep-sonnet",
        "custom_llm_provider": "anthropic", "spend": spend,
        "start_time": start_ms, "end_time": start_ms + 900, "status": "success",
        "prompt_tokens": 1200, "completion_tokens": 300, "total_tokens": 1500,
    })
    .as_object()
    .cloned()
    .unwrap_or_default()
    .into_iter()
    .collect()
}

struct LongRun {
    isolated: Isolated,
    start_ms: i64,
}

/// One run whose second model call starts seven hours after the run did, each call priced by
/// its own ledger row.
#[fixture]
async fn long_run() -> LongRun {
    let isolated = profile_store::isolated().await;
    let start_ms = chrono::Utc::now().timestamp_millis() - 9 * HOUR_MS;
    let payload = json!({"resourceSpans": [{
        "resource": {"attributes": attributes(&[("service.name", "long-service"), ("gen_ai.agent.name", AGENT)])},
        "scopeSpans": [{"scope": {"name": "profile-calls-test"}, "spans": [
            span("1000000000000001", "", start_ms, 7 * HOUR_MS + 30 * MINUTE_MS, &[
                ("gen_ai.operation.name", "invoke_agent"), ("gen_ai.agent.name", AGENT),
                ("gen_ai.prompt", "Migrate the billing service and report back"),
            ]),
            span("1000000000000002", "1000000000000001", start_ms + MINUTE_MS, 1_000, &[
                ("gen_ai.operation.name", "chat"), ("gen_ai.request.model", "sonnet"),
                ("gen_ai.response.id", "resp-early"),
            ]),
            span("1000000000000003", "1000000000000001", start_ms + 7 * HOUR_MS, 1_000, &[
                ("gen_ai.operation.name", "chat"), ("gen_ai.request.model", "sonnet"),
                ("gen_ai.response.id", "resp-late"),
            ]),
        ]}]
    }]});
    let spans = decode_otlp(&serde_json::to_vec(&payload).expect("payload"), Some("application/json")).expect("otlp");
    let tenant = Tenant {
        team_id: TEAM.into(),
        api_key_hash: KEY.into(),
        org_id: String::new(),
        user_id: "long-user".into(),
    };
    let writer = isolated.config.storage().writer();
    let database = isolated.config.storage().database();
    insert_shared_rows(&isolated.client, writer, database, InsertTable::OtelTraces, span_rows(spans, &tenant))
        .await
        .expect("spans");
    insert_rows(
        &isolated.client,
        writer,
        database,
        InsertTable::SpendLogs,
        vec![
            ledger("req-early", "resp-early", start_ms + MINUTE_MS, 0.25),
            ledger("req-late", "resp-late", start_ms + 7 * HOUR_MS, 0.5),
        ],
    )
    .await
    .expect("ledger");
    LongRun { isolated, start_ms }
}

#[rstest]
#[tokio::test]
async fn a_call_late_in_a_long_run_is_still_counted_and_priced(#[future(awt)] long_run: LongRun) {
    let profiles = &long_run.isolated.profiles;
    let runs = profiles
        .runs(
            every_team(),
            RunQuery {
                agent: AGENT.into(),
                start_ms: long_run.start_ms - HOUR_MS,
                end_ms: long_run.start_ms + 10 * HOUR_MS,
                taxonomy_id: String::new(),
                taxonomy_version: 0,
                windows: Vec::new(),
                limit: 10,
            },
        )
        .await
        .expect("runs");
    let calls = profiles.calls(every_team(), runs.clone(), 50_000).await.expect("calls");

    let totals = View {
        runs: &runs,
        calls: &calls,
        agent: AGENT,
        truncated: false,
    }
    .totals();

    assert_eq!(runs.len(), 1);
    assert_eq!(totals.requests, 2);
    assert_eq!(totals.matched_requests, 2);
    assert_eq!(totals.spend_usd, Some(0.75));
}
