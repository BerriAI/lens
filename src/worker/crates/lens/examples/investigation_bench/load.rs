use super::{Backend, Fixture};
use futures_util::{StreamExt, TryStreamExt, stream};
use litellm_lens::{Error, agent::Output, control::JobClient, evidence::Workspace, wire};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Instant};

pub async fn run(
    backend: &Arc<Backend>,
    claim: &wire::Claim,
    client: JobClient,
    concurrency: usize,
) -> Result<Value, Error> {
    let Fixture { sample, parts, .. } = &backend.fixture;
    let workspace = Workspace::new(sample.executions.clone(), client);
    let started = Instant::now();
    let reading = stream::iter(sample.executions.iter().map(|execution| {
        let workspace = &workspace;
        async move {
            let fingerprint = workspace.fingerprint(execution).await?;
            let request = serde_json::from_value(
                json!({"action":"search","execution_id":execution.id,"query":"USER:"}),
            )?;
            let response = workspace.respond(&request).await?;
            Ok::<_, Error>((execution.id.clone(), fingerprint, response))
        }
    }))
    .buffer_unordered(concurrency)
    .try_collect::<Vec<_>>()
    .await?;
    let reading_ms = started.elapsed().as_millis();
    let reading_pages = backend.metrics.lock().unwrap().pages;
    let checking = Instant::now();
    let checked = stream::iter(sample.executions.iter().zip(parts).map(|(execution, parts)| {
        let workspace = &workspace;
        async move {
            let original = &parts[2].content;
            let tail = original.rsplit_once("\nUSER:").unwrap().1;
            let lines: Vec<_> = tail.lines().collect();
            let quotes = [lines[0], lines[1], lines[2], "ASSISTANT:"];
            let extraction: wire::Extraction = serde_json::from_value(json!({"observations":[{
                "check_id":"shipment", "kind":"issue", "summary":"Mechanical citation validation workload, not a model judgment",
                "evidence":quotes.iter().map(|quote| json!({"execution_id":execution.id,"span_id":"span-2","quote":quote})).collect::<Vec<_>>()
            }]}))?;
            let error = extraction.validate(claim, workspace).await?;
            if error.is_some() { return Err(Error::InvalidRequest); }
            Ok::<_, Error>((execution.id.clone(), extraction))
        }
    })).buffer_unordered(concurrency).try_collect::<Vec<_>>().await?;
    let checking_ms = checking.elapsed().as_millis();
    let mut reading = reading;
    let mut checked = checked;
    reading.sort_by(|a, b| a.0.cmp(&b.0));
    checked.sort_by(|a, b| a.0.cmp(&b.0));
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(reading, checked))?)
    );
    let pages = backend.metrics.lock().unwrap().pages;
    Ok(
        json!({"mode":"evidence", "model":null,"sessions":sample.executions.len(),"fixture_sha256":backend.fixture.sha256,
        "fixture_tokens_cl100k":backend.fixture.tokens,"concurrency":concurrency,"io_delay_ms":backend.io_delay.as_millis(),
        "wall_ms":started.elapsed().as_millis(),"phases_ms":{"Reading evidence":reading_ms,"Checking quotes":checking_ms},
        "content_pages":pages,"phase_pages":{"Reading evidence":reading_pages,"Checking quotes":pages-reading_pages},
        "model_calls":[],"paid_catalog_cost_usd":0,"response_sha256":digest,"validated_quotes":sample.executions.len()*4,
        "quality":null,"scope":"Evidence subsystem only. No inference, confidence evaluation or independent quality assessment. Reading fingerprints full traces and searches for a late literal, then Checking validates four exact citations per session using the production Output validator."}),
    )
}
