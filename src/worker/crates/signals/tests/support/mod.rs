#![allow(
    dead_code,
    reason = "Shared fixtures are consumed by separate integration-test targets"
)]

use chrono::{DateTime, Utc};
use lens_contract::{
    feedback::TraceIdentity,
    investigations::Scope,
    signals::{SignalAttempt, SignalConfig, StoredTraceSignal},
    worker::{Execution, ExecutionContent, Sample, TracePart},
};
use lens_signals::{
    DecisionRequest, Decisions, DecisionsError, RepositoryError, SignalReader, SignalRepository,
    SourceError,
};
use rstest::fixture;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[fixture]
pub fn execution() -> Execution {
    serde_json::from_value(json!({"id":"id","source":"traces","trace_id":"trace","trace_ref":"ref","team_id":"team","name":"agent","start_time":"","span_count":2})).unwrap()
}
#[fixture]
pub fn config() -> SignalConfig {
    serde_json::from_value(json!({"model":"signals","signals":[{"id":"a","name":"First","question":"First question?"},{"id":"b","name":"Second","question":"Second question?"}]})).unwrap()
}
#[fixture]
pub fn scope() -> Scope {
    Scope {
        all_teams: true,
        ..Default::default()
    }
}
#[fixture]
pub fn now() -> DateTime<Utc> {
    "2026-01-01T12:00:00Z".parse().unwrap()
}

pub fn part(content: impl Into<String>) -> TracePart {
    TracePart {
        execution_id: "id".into(),
        span_id: "span".into(),
        parent_span_id: "".into(),
        name: "question".into(),
        kind: "user".into(),
        start_time: "".into(),
        end_time: "".into(),
        content: content.into(),
        truncated: false,
    }
}
pub fn page(parts: Vec<TracePart>, next: Option<&str>) -> ExecutionContent {
    ExecutionContent {
        execution: execution(),
        parts,
        next_cursor: next.map(str::to_owned),
        partial: false,
    }
}
pub fn sample(count: usize, next: Option<&str>) -> Sample {
    Sample {
        eligible: count as i64,
        selected: count as i64,
        next_offset: None,
        next_cursor: next.map(str::to_owned),
        executions: (0..count)
            .map(|i| Execution {
                id: format!("id-{i}"),
                trace_id: format!("trace-{i}"),
                ..execution()
            })
            .collect(),
    }
}

#[derive(Debug)]
pub struct ScanCall {
    pub scope: Scope,
    pub start: i64,
    pub end: i64,
    pub limit: u32,
    pub cursor: String,
}
pub struct Reader {
    pub contents: BTreeMap<String, ExecutionContent>,
    pub samples: BTreeMap<String, Sample>,
    pub content_calls: Mutex<Vec<String>>,
    pub scans: Mutex<Vec<ScanCall>>,
    pub fail_content: bool,
    pub fail_sample: bool,
}
#[fixture]
pub fn reader() -> Reader {
    Reader {
        contents: BTreeMap::from([("".into(), page(vec![part("user text")], None))]),
        samples: BTreeMap::from([("".into(), sample(1, None))]),
        content_calls: Mutex::default(),
        scans: Mutex::default(),
        fail_content: false,
        fail_sample: false,
    }
}
impl SignalReader for Reader {
    async fn content(
        &self,
        _: &Scope,
        _: &Execution,
        cursor: &str,
    ) -> Result<ExecutionContent, SourceError> {
        self.content_calls.lock().unwrap().push(cursor.into());
        if self.fail_content {
            return Err(SourceError(
                std::io::Error::other("secret-storage-detail").into(),
            ));
        }
        Ok(self
            .contents
            .get(cursor)
            .expect("unexpected content cursor")
            .clone())
    }
    async fn sample(
        &self,
        scope: &Scope,
        start: i64,
        end: i64,
        page_size: u32,
        cursor: &str,
    ) -> Result<Sample, SourceError> {
        self.scans.lock().unwrap().push(ScanCall {
            scope: scope.clone(),
            start,
            end,
            limit: page_size,
            cursor: cursor.into(),
        });
        if self.fail_sample {
            return Err(SourceError(
                std::io::Error::other("secret-storage-detail").into(),
            ));
        }
        Ok(self
            .samples
            .get(cursor)
            .expect("unexpected sample cursor")
            .clone())
    }
}

pub struct Completion {
    pub response: Value,
    pub fail: bool,
    pub calls: Mutex<Vec<DecisionRequest>>,
    pub active: AtomicUsize,
    pub maximum: AtomicUsize,
}
#[fixture]
pub fn completion() -> Completion {
    Completion {
        response: json!({"answers":{"a":{"type":"noul","noul":0.9},"b":{"type":"noul","noul":0.2}}}),
        fail: false,
        calls: Mutex::default(),
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
    }
}
impl Decisions for Completion {
    async fn complete(&self, request: &DecisionRequest) -> Result<Value, DecisionsError> {
        self.calls.lock().unwrap().push(request.clone());
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        tokio::task::yield_now().await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        if self.fail {
            return Err(DecisionsError::Unavailable(
                std::io::Error::other("secret-provider-detail").into(),
            ));
        }
        Ok(self.response.clone())
    }
}

pub struct Claim {
    pub execution: Execution,
    pub config: SignalConfig,
    pub lease: DateTime<Utc>,
    pub now: DateTime<Utc>,
}
pub struct Store {
    pub execution: Execution,
    pub config: SignalConfig,
    pub lease: DateTime<Utc>,
    pub at: DateTime<Utc>,
    pub attempt: SignalAttempt,
}
pub struct Repository {
    pub config: SignalConfig,
    pub rows: Vec<StoredTraceSignal>,
    pub config_reads: AtomicUsize,
    pub identities: Mutex<Vec<Vec<TraceIdentity>>>,
    pub claims: Mutex<Vec<Claim>>,
    pub stores: Mutex<Vec<Store>>,
    pub allow_claim: bool,
    pub fail_claim: bool,
    pub fail_store: bool,
    pub fail_config: bool,
    pub fail_rows: bool,
}
#[fixture]
pub fn repository() -> Repository {
    Repository {
        config: config(),
        rows: vec![],
        config_reads: AtomicUsize::new(0),
        identities: Mutex::default(),
        claims: Mutex::default(),
        stores: Mutex::default(),
        allow_claim: true,
        fail_claim: false,
        fail_store: false,
        fail_config: false,
        fail_rows: false,
    }
}
fn unavailable() -> RepositoryError {
    RepositoryError::Unavailable(std::io::Error::other("private-database-detail").into())
}
impl SignalRepository for Repository {
    async fn get_config(&self) -> Result<SignalConfig, RepositoryError> {
        self.config_reads.fetch_add(1, Ordering::SeqCst);
        if self.fail_config {
            return Err(unavailable());
        }
        Ok(self.config.clone())
    }
    async fn save_config(&self, _: &SignalConfig) -> Result<(), RepositoryError> {
        panic!("scheduler must not save config")
    }
    async fn traces(
        &self,
        identities: &[TraceIdentity],
    ) -> Result<Vec<StoredTraceSignal>, RepositoryError> {
        self.identities.lock().unwrap().push(identities.to_vec());
        if self.fail_rows {
            return Err(unavailable());
        }
        Ok(self.rows.clone())
    }
    async fn claim(
        &self,
        execution: &Execution,
        config: &SignalConfig,
        lease: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool, RepositoryError> {
        self.claims.lock().unwrap().push(Claim {
            execution: execution.clone(),
            config: config.clone(),
            lease,
            now,
        });
        if self.fail_claim {
            return Err(unavailable());
        }
        Ok(self.allow_claim)
    }
    async fn store(
        &self,
        execution: &Execution,
        config: &SignalConfig,
        lease: DateTime<Utc>,
        at: DateTime<Utc>,
        attempt: &SignalAttempt,
    ) -> Result<(), RepositoryError> {
        self.stores.lock().unwrap().push(Store {
            execution: execution.clone(),
            config: config.clone(),
            lease,
            at,
            attempt: attempt.clone(),
        });
        if self.fail_store {
            return Err(unavailable());
        }
        Ok(())
    }
}
