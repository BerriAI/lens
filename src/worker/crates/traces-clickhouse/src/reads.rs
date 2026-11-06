use litellm_http::Client;
use litellm_storage_clickhouse::{Error as StorageError, Query, fetch};
use litellm_traces::query::named as contracts;
use litellm_traces_cache::{StoreError, TraceStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use crate::{
    Connection, Error,
    query::conversation::{ConversationAnchor, ConversationTurns},
    query::named::{
        ListTracesParams, ListTracesRow, SpanDetail as SpanDetailQuery, SpanError, SpanErrorParams,
        SpendByResponseIdsParams, TraceIdentity, TracePageSpansParams,
    },
};

#[derive(Serialize)]
struct SpendLogInputPreviewParams {
    #[serde(flatten)]
    access: contracts::ReadAccessParams,
    trace_refs: Vec<String>,
}

#[derive(Deserialize)]
struct SpendLogInputPreviewRow {
    trace_ref: String,
    messages: String,
}

struct SpendLogInputPreviewFallback;

impl Query for SpendLogInputPreviewFallback {
    type Params = SpendLogInputPreviewParams;
    type Row = SpendLogInputPreviewRow;
    const SQL: &'static str = "SELECT \
        hex(SHA256(concat(s.team_id, char(0), s.api_key, char(0), s.trace_id))) AS trace_ref, \
        argMinIf(s.messages, tuple(s.start_time, s.request_id), s.messages != '' AND s.messages != '[]') AS messages \
        FROM spend_logs AS s FINAL \
        WHERE s.trace_id != '' AND trace_ref IN {trace_refs:Array(String)} \
          AND ({all_teams:UInt8} = 1 \
               OR ({user_id:String} != '' AND s.user = {user_id:String}) \
               OR has({team_ids:Array(String)}, s.team_id)) \
          AND s.messages != '' AND s.messages != '[]' \
        GROUP BY trace_ref";
}

fn span_trace_ref(row: &contracts::TraceSpansRow) -> String {
    let identity = format!("{}\0{}\0{}", row.team_id, row.api_key_hash, row.trace_id);
    format!("{:X}", Sha256::digest(identity.as_bytes()))
}

fn extract_user_prompt_preview(messages: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(messages).ok()?;
    let items = parsed.as_array()?;
    let mut candidate_prompts = Vec::new();
    for item in items {
        if item.get("role").and_then(|r| r.as_str()) == Some("user") {
            if let Some(content) = item.get("content").and_then(|c| c.as_str()) {
                candidate_prompts.push(content.trim());
            } else if let Some(parts) = item.get("content").and_then(|c| c.as_array()) {
                for part in parts {
                    let text = part
                        .get("text")
                        .and_then(|t| t.as_str())
                        .or_else(|| part.get("content").and_then(|c| c.as_str()));
                    if let Some(text) = text {
                        candidate_prompts.push(text.trim());
                    }
                }
            }
        }
    }
    let meaningful = candidate_prompts
        .iter()
        .copied()
        .filter(|prompt| {
            !prompt.starts_with("<environment_context>") && !prompt.starts_with("# AGENTS.md")
        })
        .map(|prompt| {
            if let Some((_, user_part)) = prompt.split_once("User prompt:\n") {
                user_part.trim()
            } else if let Some((_, user_part)) = prompt.split_once("</system-reminder>\n\n") {
                user_part.trim()
            } else if let Some((_, user_part)) = prompt.split_once("</system-reminder>") {
                user_part.trim()
            } else {
                prompt
            }
        })
        .next_back();
    let selected = meaningful.or_else(|| candidate_prompts.last().copied())?;
    let single_line: String = selected
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let preview: String = single_line.chars().take(240).collect();
    (!preview.is_empty()).then_some(preview)
}

#[derive(Deserialize)]
struct SpendLogContentRow {
    input: String,
    output: String,
}

struct SpendLogContentFallback;

impl Query for SpendLogContentFallback {
    type Params = contracts::SpanDetailParams;
    type Row = SpendLogContentRow;
    const SQL: &'static str = "SELECT \
        tupleElement(argMax(tuple(s.messages, s.response), tuple(s.start_time, s.request_id)), 1) AS input, \
        tupleElement(argMax(tuple(s.messages, s.response), tuple(s.start_time, s.request_id)), 2) AS output \
        FROM spend_logs AS s FINAL \
        CROSS JOIN ( \
            SELECT TraceId, SpanId, TeamId, ApiKeyHash, UserId, ObservationType, ParentSpanId \
            FROM otel_traces \
            WHERE TraceId = {trace_id:String} AND SpanId = {span_id:String} \
              AND (ObservationType = 'llm' OR (ObservationType = 'agent' AND ParentSpanId = '')) \
              AND ({all_teams:UInt8} = 1 \
                   OR ({user_id:String} != '' AND UserId = {user_id:String}) \
                   OR has({team_ids:Array(String)}, TeamId)) \
              AND ({trace_ref:String} = '' OR \
                   hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId))) = {trace_ref:String}) \
            LIMIT 1 \
        ) AS span \
        WHERE s.trace_id = span.TraceId AND s.trace_id != '' \
          AND s.team_id = span.TeamId AND s.api_key = span.ApiKeyHash \
          AND ({all_teams:UInt8} = 1 \
               OR ({user_id:String} != '' AND s.user = {user_id:String}) \
               OR has({team_ids:Array(String)}, s.team_id)) \
          AND (s.span_id = span.SpanId \
               OR (span.ObservationType = 'agent' AND span.ParentSpanId = '')) \
          AND (s.messages != '' OR s.response != '') \
        HAVING count() > 0";
}

struct RunCandidates;

impl Query for RunCandidates {
    type Params = ListTracesParams;
    type Row = ListTracesRow;
    const SQL: &'static str = concat!(
        "SELECT * EXCEPT (request_ids), [] AS request_ids FROM (",
        include_str!("../query/list_traces.sql"),
        ") ORDER BY start_ms DESC, trace_ref DESC"
    );
}

pub struct ClickHouseTraces {
    client: Client,
    connection: Connection,
}

impl ClickHouseTraces {
    pub fn new(client: Client, connection: Connection) -> Self {
        Self { client, connection }
    }
}

impl TraceStore for ClickHouseTraces {
    type Error = Error;

    fn source(&self) -> &str {
        self.connection.url().as_str()
    }

    async fn summary_page(
        &self,
        params: &contracts::ListTracesParams,
    ) -> Result<Option<litellm_traces_cache::SummaryPage>, StoreError<Self::Error>> {
        crate::summaries::page(&self.client, &self.connection, params)
            .await
            .map_err(StoreError::Failed)
    }

    async fn summaries(
        &self,
        access: &contracts::ReadAccessParams,
        runs: &[contracts::ListTracesRow],
    ) -> Result<Vec<litellm_traces::TraceSummary>, StoreError<Self::Error>> {
        crate::summaries::read(&self.client, &self.connection, access, runs)
            .await
            .map_err(StoreError::Failed)
    }

    async fn trace_refs(
        &self,
        params: &contracts::TraceIdentityParams,
    ) -> Result<Vec<String>, StoreError<Self::Error>> {
        fetch::<TraceIdentity>(&self.client, &self.connection, params)
            .await
            .map(|rows| rows.into_iter().map(|row| row.trace_ref).collect())
            .map_err(failed)
    }

    async fn list_runs(
        &self,
        params: &contracts::ListTracesParams,
    ) -> Result<Vec<contracts::ListTracesRow>, StoreError<Self::Error>> {
        let storage_params = ListTracesParams::from(params.clone());
        let mut rows: Vec<contracts::ListTracesRow> =
            match fetch::<RunCandidates>(&self.client, &self.connection, &storage_params).await {
                Ok(rows) => rows.into_iter().map(|row| row.0).collect(),
                Err(StorageError::ResponseTooLarge) => return Err(StoreError::TooLarge),
                Err(error) => return Err(failed(error)),
            };
        let missing_preview_ids: Vec<String> = rows
            .iter()
            .filter(|row| row.input_preview.is_empty())
            .map(|row| row.trace_ref.clone())
            .collect();
        if !missing_preview_ids.is_empty() {
            let fallback_params = SpendLogInputPreviewParams {
                access: params.access.clone(),
                trace_refs: missing_preview_ids,
            };
            if let Ok(fallback_rows) = fetch::<SpendLogInputPreviewFallback>(
                &self.client,
                &self.connection,
                &fallback_params,
            )
            .await
            {
                let previews: HashMap<String, String> = fallback_rows
                    .into_iter()
                    .filter_map(|row| {
                        extract_user_prompt_preview(&row.messages)
                            .map(|preview| (row.trace_ref, preview))
                    })
                    .collect();
                for row in &mut rows {
                    if row.input_preview.is_empty()
                        && let Some(preview) = previews.get(&row.trace_ref)
                    {
                        row.input_preview = preview.clone();
                    }
                }
            }
        }
        Ok(rows)
    }

    async fn trace_spans(
        &self,
        params: &contracts::TraceSpansParams,
        snapshot_ms: u64,
    ) -> Result<Vec<contracts::TraceSpansRow>, StoreError<Self::Error>> {
        let mut rows = crate::span_batches::read_spans(
            &self.client,
            &self.connection,
            params.clone(),
            snapshot_ms,
        )
        .await?;
        if rows.iter().all(|row| row.input_preview.is_empty()) && !rows.is_empty() {
            let fallback_params = SpendLogInputPreviewParams {
                access: params.access.clone(),
                trace_refs: rows.iter().map(span_trace_ref).collect(),
            };
            if let Ok(fallback_rows) = fetch::<SpendLogInputPreviewFallback>(
                &self.client,
                &self.connection,
                &fallback_params,
            )
            .await
            {
                for fallback in fallback_rows {
                    if let Some(preview) = extract_user_prompt_preview(&fallback.messages)
                        && let Some(row) = rows
                            .iter_mut()
                            .filter(|row| span_trace_ref(row) == fallback.trace_ref)
                            .min_by_key(|row| !row.parent_span_id.is_empty())
                    {
                        row.input_preview = preview;
                    }
                }
            }
        }
        Ok(rows)
    }

    async fn run_spans(
        &self,
        params: &contracts::TracePageSpansParams,
        snapshot_ms: u64,
    ) -> Result<Vec<contracts::TraceSpansRow>, StoreError<Self::Error>> {
        let mut rows = crate::span_batches::read_list_spans(
            &self.client,
            &self.connection,
            TracePageSpansParams::from(params.clone()),
            snapshot_ms,
        )
        .await?;
        let mut empty_traces: HashMap<String, bool> = HashMap::new();
        for row in &rows {
            let entry = empty_traces.entry(span_trace_ref(row)).or_insert(true);
            if !row.input_preview.is_empty() {
                *entry = false;
            }
        }
        let missing_preview_trace_ids: Vec<String> = empty_traces
            .into_iter()
            .filter_map(|(id, is_empty)| is_empty.then_some(id))
            .collect();
        if !missing_preview_trace_ids.is_empty() {
            let fallback_params = SpendLogInputPreviewParams {
                access: params.access.clone(),
                trace_refs: missing_preview_trace_ids,
            };
            if let Ok(fallback_rows) = fetch::<SpendLogInputPreviewFallback>(
                &self.client,
                &self.connection,
                &fallback_params,
            )
            .await
            {
                let previews: HashMap<String, String> = fallback_rows
                    .into_iter()
                    .filter_map(|row| {
                        extract_user_prompt_preview(&row.messages)
                            .map(|preview| (row.trace_ref, preview))
                    })
                    .collect();
                for (trace_ref, preview) in previews {
                    if let Some(root_row) = rows
                        .iter_mut()
                        .find(|r| span_trace_ref(r) == trace_ref && r.parent_span_id.is_empty())
                    {
                        root_row.input_preview = preview;
                    } else if let Some(first_row) =
                        rows.iter_mut().find(|r| span_trace_ref(r) == trace_ref)
                    {
                        first_row.input_preview = preview;
                    }
                }
            }
        }
        Ok(rows)
    }

    async fn spend(
        &self,
        params: &contracts::SpendByResponseIdsParams,
    ) -> Result<Vec<contracts::SpendByResponseIdsRow>, StoreError<Self::Error>> {
        crate::span_batches::read_spend(
            &self.client,
            &self.connection,
            SpendByResponseIdsParams::from(params.clone()),
        )
        .await
    }

    async fn span_detail(
        &self,
        params: &contracts::SpanDetailParams,
    ) -> Result<Option<contracts::SpanDetailRow>, StoreError<Self::Error>> {
        let mut detail =
            match fetch::<SpanDetailQuery>(&self.client, &self.connection, params).await {
                Ok(rows) => rows.into_iter().next(),
                Err(error) => return Err(failed(error)),
            };
        if let Some(row) = detail.as_mut()
            && (row.input.is_empty() || row.output.is_empty())
            && let Ok(mut fallback) =
                fetch::<SpendLogContentFallback>(&self.client, &self.connection, params).await
            && let Some(content) = fallback.pop()
        {
            if row.input.is_empty() && !content.input.is_empty() {
                row.input = content.input;
            }
            if row.output.is_empty() && !content.output.is_empty() {
                row.output = content.output;
            }
        }
        Ok(detail)
    }

    async fn span_error(
        &self,
        params: &contracts::SpanErrorParams,
    ) -> Result<Option<contracts::SpanErrorRow>, StoreError<Self::Error>> {
        let storage_params = SpanErrorParams::from(params.clone());
        match fetch::<SpanError>(&self.client, &self.connection, &storage_params).await {
            Ok(rows) => Ok(rows.into_iter().next().map(|row| row.0)),
            Err(error) => Err(failed(error)),
        }
    }

    async fn conversation_anchor(
        &self,
        params: &contracts::TraceConversationAnchorParams,
    ) -> Result<Option<contracts::TraceConversationAnchor>, StoreError<Self::Error>> {
        fetch::<ConversationAnchor>(&self.client, &self.connection, params)
            .await
            .map(|rows| rows.into_iter().next().map(|row| row.0))
            .map_err(failed)
    }

    async fn conversation_turns(
        &self,
        params: &contracts::TraceConversationTurnsParams,
    ) -> Result<Vec<contracts::TraceConversationRow>, StoreError<Self::Error>> {
        match fetch::<ConversationTurns>(&self.client, &self.connection, params).await {
            Ok(rows) => Ok(rows.into_iter().map(|row| row.0).collect()),
            Err(StorageError::ResponseTooLarge) => Err(StoreError::TooLarge),
            Err(error) => Err(failed(error)),
        }
    }
}

fn failed(error: StorageError) -> StoreError<Error> {
    StoreError::Failed(Error::Storage(error))
}

#[cfg(test)]
mod tests {
    use super::extract_user_prompt_preview;

    #[test]
    fn extracts_direct_user_string_prompt() {
        let messages = serde_json::json!([
            {"role": "developer", "content": "You are a helpful assistant."},
            {"role": "user", "content": "Write a rust CLI tool"}
        ])
        .to_string();
        assert_eq!(
            extract_user_prompt_preview(&messages),
            Some("Write a rust CLI tool".to_owned())
        );
    }

    #[test]
    fn extracts_nested_input_text_and_strips_title_generation_preamble() {
        let messages = serde_json::json!([
            {
                "role": "user",
                "content": [{"type": "input_text", "text": "<environment_context>\n2026-11-03\n</environment_context>"}]
            },
            {
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": "Generate a concise title.\n\nUser prompt:\nWrite short summary for business_plan.md\n\nShow draft."
                }]
            }
        ])
        .to_string();
        assert_eq!(
            extract_user_prompt_preview(&messages),
            Some("Write short summary for business_plan.md Show draft.".to_owned())
        );
    }

    #[test]
    fn caps_length_at_240_chars_and_flattens_whitespace() {
        let long_prompt = "word ".repeat(100);
        let messages = serde_json::json!([{"role": "user", "content": long_prompt}]).to_string();
        let preview = extract_user_prompt_preview(&messages).unwrap();
        assert!(preview.len() <= 240);
        assert!(!preview.contains('\n'));
    }
}
