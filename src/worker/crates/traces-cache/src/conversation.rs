use std::sync::Arc;

use litellm_traces::{
    TraceConversationPage, TraceConversationTurn, iso_time,
    query::named::{
        ReadAccessParams, TraceConversationAnchor, TraceConversationAnchorParams,
        TraceConversationRow, TraceConversationTurnsParams,
    },
    request::{CONVERSATION_PAGE_SIZE_MAX, TRACE_PAGE_SIZE_MIN},
    to_ui_content,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    ReadError, StoreError, TraceReader, TraceStore,
    cursor::{decode_cursor, encode_cursor},
    reader::{map_store_error, now_ms, reference},
};

#[derive(Deserialize, Serialize)]
struct ConversationPosition {
    anchor: String,
    snapshot_ms: u64,
    start_ns: i64,
    trace_ref: String,
    span_id: String,
}

fn anchor_key<E>(
    source: &str,
    access: &ReadAccessParams,
    anchor: &TraceConversationAnchor,
) -> Result<String, ReadError<E>> {
    let encoded = serde_json::to_vec(&(source, access, anchor))
        .map_err(|error| ReadError::Encode(Arc::new(error)))?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

impl TraceReader {
    pub async fn get_conversation<S: TraceStore>(
        &self,
        store: &S,
        access: &ReadAccessParams,
        trace_id: &str,
        trace_ref: &str,
        cursor: Option<&str>,
        page_size: u16,
    ) -> Result<Option<TraceConversationPage>, ReadError<S::Error>> {
        if !(TRACE_PAGE_SIZE_MIN..=CONVERSATION_PAGE_SIZE_MAX).contains(&page_size) {
            return Err(ReadError::InvalidParameters);
        }
        let position: Option<ConversationPosition> = cursor
            .map(|cursor| decode_cursor(cursor, "conversation"))
            .transpose()?;
        let now = now_ms();
        let snapshot_ms = position
            .as_ref()
            .map_or(now, |position| position.snapshot_ms);
        if snapshot_ms == 0 || snapshot_ms > now {
            return Err(ReadError::InvalidCursor("conversation"));
        }
        let Some(trace_ref) = reference(store, access, trace_id, trace_ref).await? else {
            return Ok(None);
        };
        let Some(anchor) = store
            .conversation_anchor(&TraceConversationAnchorParams {
                access: access.clone(),
                trace_id: trace_id.to_owned(),
                trace_ref,
                snapshot_ms,
            })
            .await
            .map_err(map_store_error)?
        else {
            return Ok(None);
        };
        let key = anchor_key(store.source(), access, &anchor)?;
        if position
            .as_ref()
            .is_some_and(|position| position.anchor != key)
        {
            return Err(ReadError::TraceChanged);
        }
        if anchor.session_id.is_empty() {
            return Ok(Some(TraceConversationPage {
                turns: Vec::new(),
                next_cursor: None,
            }));
        }
        let mut params = TraceConversationTurnsParams {
            access: access.clone(),
            team_id: anchor.team_id,
            api_key_hash: anchor.api_key_hash,
            session_id: anchor.session_id,
            current_trace_id: trace_id.to_owned(),
            before_ns: anchor.start_ns,
            snapshot_ms,
            has_cursor: u8::from(position.is_some()),
            after_start_ns: position.as_ref().map_or(0, |position| position.start_ns),
            after_trace_ref: position
                .as_ref()
                .map_or_else(String::new, |position| position.trace_ref.clone()),
            after_span_id: position
                .as_ref()
                .map_or_else(String::new, |position| position.span_id.clone()),
            limit: u32::from(page_size) + 1,
        };
        let rows = loop {
            match store.conversation_turns(&params).await {
                Err(StoreError::TooLarge) if params.limit > 2 => {
                    params.limit = (params.limit / 2).max(2);
                }
                result => break result.map_err(map_store_error)?,
            }
        };
        conversation_page(
            &rows,
            (params.limit - 1) as usize,
            &key,
            snapshot_ms,
            self.response_bytes,
        )
        .map(Some)
    }
}

fn conversation_page<E>(
    rows: &[TraceConversationRow],
    limit: usize,
    anchor: &str,
    snapshot_ms: u64,
    max_bytes: usize,
) -> Result<TraceConversationPage, ReadError<E>> {
    let mut count = rows.len().min(limit);
    loop {
        let turns = rows[..count]
            .iter()
            .map(|row| TraceConversationTurn {
                trace_id: row.trace_id.clone(),
                trace_ref: row.trace_ref.clone(),
                span_id: row.span_id.clone(),
                start_time: iso_time(row.start_ns.div_euclid(1_000_000)),
                input_ui: to_ui_content(&row.input),
                output_ui: to_ui_content(&row.output),
                input: row.input.clone(),
                output: row.output.clone(),
            })
            .collect();
        let next_cursor = rows
            .get(count.saturating_sub(1))
            .filter(|_| count > 0 && count < rows.len())
            .map(|last| {
                encode_cursor(&ConversationPosition {
                    anchor: anchor.to_owned(),
                    snapshot_ms,
                    start_ns: last.start_ns,
                    trace_ref: last.trace_ref.clone(),
                    span_id: last.span_id.clone(),
                })
            });
        let page = TraceConversationPage { turns, next_cursor };
        if serde_json::to_vec(&page)
            .map_err(|error| ReadError::Encode(Arc::new(error)))?
            .len()
            <= max_bytes
        {
            return Ok(page);
        }
        if count <= 1 {
            return Err(ReadError::TooLarge);
        }
        count /= 2;
    }
}
