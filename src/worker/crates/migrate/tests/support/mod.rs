use lens_migrate::{Error, LegacySnapshot};
use rstest::fixture;
use serde_json::{Value, json};

#[fixture]
pub fn source() -> Value {
    let public: Value = serde_json::from_str(include_str!(
        "../../../contract/tests/fixtures/investigations_public.json"
    ))
    .unwrap();
    let dataset = json!({"id":"dataset","name":"Saved cases","agent_name":"agent","team_id":"alpha","created_at":"2026-01-15T00:00:00Z","revision":1,"created_by":"owner","cases":[{"id":"saved-case","messages":[{"role":"user","content":"Find the order"}],"reply":"Found order 42","tool_calls":[{"name":"lookup","arguments":"{\"order_id\":42}"}],"expected":"Order found","included":false,"source":{"trace_id":"trace","trace_ref":"backend-key","span_id":"span","finding_id":"finding","lens_id":"lens"},"agent_version":"v1"}]});
    let mut second = dataset.clone();
    second["revision"] = json!(2);
    let mut archived = public["job"].clone();
    archived["id"] = json!("archive");
    let job: lens_contract::worker::Job = serde_json::from_value(archived.clone()).unwrap();
    let criteria = lens_investigations::criteria_key(&job.settings).unwrap();
    json!({
        "lenses":[{"id":"lens","version":42,"data":public["filled_lens"],"due_at":"2026-01-15T01:02:03.123456"}],
        "runs":[{"id":"archive","lens_id":"lens","created_at":"2026-01-15T00:00:00","data":archived}],
        "reviews":[{"lens_id":"lens","criteria_key":criteria,"execution_id":public["review"]["execution_id"],"data":public["review"]}],
        "workers":[{"id":public["worker"]["id"],"token_hash":"keep-hash-byte-exact","data":public["worker"]}],
        "ingestion_keys":[{"id":"key-hash","data":{"id":"key-hash","name":"Tracing","tenant":{"team_id":"alpha","user_id":"owner","org_id":"organization","api_key_hash":"source-key-hash"},"created_at":"2026-01-15T00:00:00Z","expires_at":1800000000}}],
        "datasets":[{"id":"dataset","revision":2,"created_at":"2026-01-17T00:00:00","data":second},{"id":"dataset","revision":1,"created_at":"2026-01-16T00:00:00","data":dataset}],
        "signal_configs":[{"id":"global","data":{}}],
        "trace_signals":[{"trace_id":"trace","trace_ref":"backend-key","config_key":"keep-config-key","span_count":3,"claimed_until":"2026-01-15T00:00:00","classified_at":null,"data":{"status":"pending","scores":{},"model":"signals","error":""}}]
    })
}

#[fixture]
pub fn many_lenses(mut source: Value) -> Value {
    let original = source["lenses"][0].clone();
    source["lenses"]
        .as_array_mut()
        .unwrap()
        .extend((0..270).map(|index| {
            let id = format!("lens-{index:03}");
            let mut row = original.clone();
            row["id"] = json!(id);
            row["data"]["id"] = json!(id);
            row
        }));
    source
}

pub fn plan(source: Value) -> Result<lens_migrate::Plan, Error> {
    serde_json::from_value::<LegacySnapshot>(source)?.plan()
}
