use std::{sync::Arc, time::Duration};

use axum::{
    Json,
    body::{Body, Bytes, to_bytes},
    extract::State as AppState,
    http::{HeaderMap, StatusCode},
};
use lens_contract::ledger::{
    LEDGER_VERSION, LedgerBatch, LedgerCapabilities, LedgerEnvelope, LedgerResult, LedgerResults,
    LedgerStatus, PayloadUpload, SUPPORTED_LEDGER_VERSIONS,
};
use lens_payloads::{Field, ObjectKey, Sweep, sha256_hex};
use litellm_traces_clickhouse::{
    LedgerRow, PayloadRefRow, insert_ledger, insert_payload_ref, ledger_row,
};

use crate::{Error, State, ingest::MAX_UPLOAD_BYTES};

pub const MAX_LEDGER_EVENTS: usize = 1000;
const BODY_TIMEOUT: Duration = Duration::from_secs(10);
const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadLimits {
    pub inline_bytes: usize,
    pub preview_bytes: usize,
}

impl PayloadLimits {
    pub const WITHOUT_STORE: Self = Self {
        inline_bytes: 1024 * 1024,
        preview_bytes: 1024 * 1024,
    };
    pub const WITH_STORE: Self = Self {
        inline_bytes: 64 * 1024,
        preview_bytes: 4 * 1024,
    };

    pub fn from_lookup(read: impl Fn(&str) -> Option<String>, store: bool) -> Result<Self, Error> {
        let defaults = if store {
            Self::WITH_STORE
        } else {
            Self::WITHOUT_STORE
        };
        let bytes = |name: &'static str, default: usize| match read(name) {
            None => Ok(default),
            Some(value) => value
                .parse::<usize>()
                .ok()
                .filter(|bytes| *bytes >= lens_payloads::MIN_PREVIEW_BYTES)
                .ok_or(Error::Configuration(name)),
        };
        let inline_bytes = bytes("LENS_PAYLOAD_INLINE_BYTES", defaults.inline_bytes)?;
        let preview_bytes = bytes("LENS_PAYLOAD_PREVIEW_BYTES", defaults.preview_bytes)?;
        if preview_bytes > inline_bytes {
            return Err(Error::Configuration(
                "LENS_PAYLOAD_PREVIEW_BYTES must not exceed LENS_PAYLOAD_INLINE_BYTES",
            ));
        }
        Ok(Self {
            inline_bytes,
            preview_bytes,
        })
    }

    pub fn capabilities(self, payload_store: bool) -> LedgerCapabilities {
        LedgerCapabilities {
            versions: SUPPORTED_LEDGER_VERSIONS.to_vec(),
            payload_store,
            inline_bytes: self.inline_bytes as u64,
            preview_bytes: self.preview_bytes as u64,
        }
    }
}

pub fn project(batch: &LedgerBatch, limits: PayloadLimits) -> (Vec<LedgerRow>, Vec<LedgerResult>) {
    let outcomes: Vec<_> = batch
        .events
        .iter()
        .map(|event| {
            (
                event.event_id.clone(),
                ledger_row(&event.event_id, &event.record, limits.inline_bytes),
            )
        })
        .collect();
    let results = outcomes
        .iter()
        .map(|(event_id, outcome)| LedgerResult {
            event_id: event_id.clone(),
            status: match outcome {
                Ok(_) => LedgerStatus::Stored,
                Err(_) => LedgerStatus::Rejected,
            },
            reason: outcome.as_ref().err().copied(),
        })
        .collect();
    let rows = outcomes
        .into_iter()
        .filter_map(|(_, outcome)| outcome.ok())
        .collect();
    (rows, results)
}

pub fn parse_batch(body: &[u8]) -> Result<LedgerBatch, Error> {
    let envelope: LedgerEnvelope =
        serde_json::from_slice(body).map_err(|_| Error::InvalidRequest)?;
    if !SUPPORTED_LEDGER_VERSIONS.contains(&envelope.version) {
        return Err(Error::UnsupportedLedgerVersion {
            supported: SUPPORTED_LEDGER_VERSIONS,
        });
    }
    let batch: LedgerBatch = serde_json::from_slice(body).map_err(|_| Error::InvalidRequest)?;
    if batch.events.len() > MAX_LEDGER_EVENTS {
        return Err(Error::TooLarge);
    }
    Ok(batch)
}

pub fn parse_upload(body: &[u8], prefix: &str) -> Result<(PayloadUpload, Field, ObjectKey), Error> {
    let envelope: LedgerEnvelope =
        serde_json::from_slice(body).map_err(|_| Error::InvalidRequest)?;
    if envelope.version != LEDGER_VERSION {
        return Err(Error::UnsupportedLedgerVersion {
            supported: SUPPORTED_LEDGER_VERSIONS,
        });
    }
    let upload: PayloadUpload = serde_json::from_slice(body).map_err(|_| Error::InvalidRequest)?;
    let field = Field::parse(&upload.field).ok_or(Error::InvalidRequest)?;
    if upload.content.len() as u64 != upload.bytes
        || sha256_hex(upload.content.as_bytes()) != upload.sha256
    {
        return Err(Error::Payload(lens_payloads::Error::Integrity));
    }
    let key = ObjectKey::at_millis(
        prefix,
        upload.event_time_ms,
        &upload.team_id,
        &upload.api_key,
        &upload.event_id,
        field,
    )?;
    Ok((upload, field, key))
}

async fn read_body(body: Body) -> Result<Bytes, Error> {
    tokio::time::timeout(BODY_TIMEOUT, to_bytes(body, MAX_UPLOAD_BYTES))
        .await
        .map_err(|_| Error::Unavailable)?
        .map_err(|_| Error::TooLarge)
}

pub(crate) async fn receive(
    AppState(state): AppState<Arc<State>>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<LedgerResults>, Error> {
    state.authorize_service(&headers)?;
    state.require_storage()?;
    let permit = state
        .export_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::Unavailable)?;
    let body = read_body(body).await?;
    tokio::spawn(async move {
        let _permit = permit;
        let batch = parse_batch(&body)?;
        let (rows, results) = project(&batch, state.payload_limits);
        let storage = state.storage.config.storage();
        insert_ledger(
            &state.storage.client,
            storage.writer(),
            storage.database(),
            &rows,
        )
        .await?;
        Ok(Json(LedgerResults {
            version: LEDGER_VERSION,
            results,
        }))
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

pub(crate) async fn upload(
    AppState(state): AppState<Arc<State>>,
    headers: HeaderMap,
    body: Body,
) -> Result<StatusCode, Error> {
    state.authorize_service(&headers)?;
    state.require_storage()?;
    let payloads = state
        .storage
        .payloads()
        .cloned()
        .ok_or(Error::PayloadStoreDisabled)?;
    let permit = state
        .export_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::Unavailable)?;
    let body = read_body(body).await?;
    tokio::spawn(async move {
        let _permit = permit;
        let store = payloads.store();
        let (upload, field, key) = parse_upload(&body, store.prefix())?;
        store
            .put(&key, upload.content.into_bytes(), &upload.sha256)
            .await?;
        let storage = state.storage.config.storage();
        insert_payload_ref(
            &state.storage.client,
            storage.writer(),
            storage.database(),
            &PayloadRefRow {
                team_id: upload.team_id,
                api_key: upload.api_key,
                event_id: upload.event_id,
                field: field.name(),
                event_time_ms: upload.event_time_ms,
                object_key: key.as_str().to_owned(),
                bytes: upload.bytes,
                sha256: upload.sha256,
            },
        )
        .await?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

pub async fn sweep_payloads(state: Arc<State>) {
    let Some(payloads) = state.storage.payloads().cloned() else {
        return std::future::pending().await;
    };
    let retention_days = u64::from(state.storage.config.retention_days());
    loop {
        match payloads
            .store()
            .sweep(chrono::Utc::now(), retention_days)
            .await
        {
            Ok(Sweep { deleted, failed }) => {
                if deleted > 0 {
                    tracing::info!(deleted, "deleted payload objects past retention");
                }
                if failed > 0 {
                    tracing::warn!(
                        failed,
                        "payload objects past retention were not deleted; retrying later"
                    );
                }
            }
            Err(error) => tracing::warn!(%error, "payload retention sweep failed; retrying later"),
        }
        tokio::time::sleep(SWEEP_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use super::*;

    fn record(start_time: i64) -> serde_json::Value {
        json!({"request_id": "req", "team_id": "team", "api_key": "hash", "spend": 0.25, "start_time": start_time})
    }

    #[rstest]
    fn one_invalid_event_does_not_block_the_others() {
        let batch: LedgerBatch = serde_json::from_value(json!({
            "version": 1,
            "events": [
                {"event_id": "good", "record": record(1_767_225_600_000)},
                {"event_id": "bad", "record": record(0)},
            ],
        }))
        .unwrap();

        let (rows, results) = project(&batch, PayloadLimits::WITHOUT_STORE);

        assert_eq!(rows.len(), 1);
        assert_eq!(
            serde_json::to_value(results).unwrap(),
            json!([
                {"event_id": "good", "status": "stored"},
                {"event_id": "bad", "status": "rejected", "reason": "missing_event_time"},
            ])
        );
    }

    #[rstest]
    #[case::future(json!({"version": 2, "events": []}), StatusCode::UNPROCESSABLE_ENTITY)]
    #[case::malformed(json!({"version": 1, "events": [{"event_id": "e"}]}), StatusCode::BAD_REQUEST)]
    fn batches_outside_version_one_are_refused_whole(
        #[case] body: serde_json::Value,
        #[case] status: StatusCode,
    ) {
        assert_eq!(
            parse_batch(body.to_string().as_bytes())
                .unwrap_err()
                .status(),
            status
        );
    }

    fn upload(content: &str, sha256: String) -> serde_json::Value {
        json!({
            "version": 1, "event_id": "evt", "field": "messages", "team_id": "team",
            "api_key": "hash", "event_time_ms": 1_767_225_600_000_i64,
            "bytes": content.len(), "sha256": sha256, "content": content,
        })
    }

    #[rstest]
    fn upload_key_is_scoped_to_the_declared_tenant() {
        let content = "[\"雪\"]";
        let (_, field, key) = parse_upload(
            upload(content, sha256_hex(content.as_bytes()))
                .to_string()
                .as_bytes(),
            "",
        )
        .unwrap();

        assert_eq!(field, Field::Messages);
        assert!(key.is_for("", "team", "hash", "evt", Field::Messages));
        assert!(!key.is_for("", "other", "hash", "evt", Field::Messages));
    }

    #[rstest]
    fn upload_with_a_wrong_checksum_is_a_permanent_failure() {
        let error =
            parse_upload(upload("[]", sha256_hex(b"{}")).to_string().as_bytes(), "").unwrap_err();
        assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[rstest]
    #[case::store_defaults(true, &[], Ok(PayloadLimits::WITH_STORE))]
    #[case::inline_defaults(false, &[], Ok(PayloadLimits::WITHOUT_STORE))]
    #[case::below_minimum(true, &[("LENS_PAYLOAD_INLINE_BYTES", "10")], Err(()))]
    #[case::preview_above_inline(true, &[("LENS_PAYLOAD_PREVIEW_BYTES", "131072")], Err(()))]
    fn limits_follow_store_and_operator_settings(
        #[case] store: bool,
        #[case] values: &[(&str, &str)],
        #[case] expected: Result<PayloadLimits, ()>,
    ) {
        let read = |name: &str| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        };
        assert_eq!(
            PayloadLimits::from_lookup(read, store).map_err(|_| ()),
            expected
        );
    }
}
