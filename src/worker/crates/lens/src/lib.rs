pub mod activity;
pub mod agent;
pub mod api;
pub mod auth;
pub mod config;
pub mod control;
mod error;
pub mod eval_judge;
pub mod eval_runtime;
pub mod eval_scoring;
pub mod evidence;
pub mod gateway;
pub mod grouping;
mod ingest;
pub mod journal;
pub mod local;
pub mod local_credentials;
pub mod model;
pub mod pipeline;
pub mod sandbox;
pub mod setup;
pub mod signals;
pub mod slack;
mod storage;
pub mod worker;

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::State as AppState,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
pub use error::Error;
use litellm_traces_clickhouse::InsertTable;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
pub use storage::{SampleRequest, SourceReader, Storage};
use tokio::sync::Semaphore;

pub use lens_contract::worker as wire;
pub use storage::FeedbackApi;

const READ_QUEUE_WAIT: Duration = Duration::from_secs(10);

pub struct State {
    pub credentials: Arc<auth::Credentials>,
    pub storage: Storage,
    pub schema_ready: AtomicBool,
    service_token: Option<String>,
    ingest_slots: Arc<Semaphore>,
    read_slots: Arc<Semaphore>,
    export_slots: Arc<Semaphore>,
}

impl State {
    pub fn connected(storage: Storage, service_token: String) -> Self {
        Self::with_service_token(storage, Some(service_token))
    }

    pub fn standalone(storage: Storage) -> Self {
        Self::with_service_token(storage, None)
    }

    fn with_service_token(storage: Storage, service_token: Option<String>) -> Self {
        Self {
            credentials: Arc::new(auth::Credentials::default()),
            storage,
            schema_ready: AtomicBool::new(false),
            service_token,
            ingest_slots: Arc::new(Semaphore::new(2)),
            read_slots: Arc::new(Semaphore::new(8)),
            export_slots: Arc::new(Semaphore::new(2)),
        }
    }

    fn authorize_service(&self, headers: &HeaderMap) -> Result<(), Error> {
        auth::authorize_service(
            headers,
            self.service_token.as_deref().ok_or(Error::Unauthorized)?,
        )
    }

    fn require_storage(&self) -> Result<(), Error> {
        if self.schema_ready.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(Error::Unavailable)
        }
    }
}

async fn wait_for_read_slot<P>(
    acquire: impl Future<Output = Result<P, tokio::sync::AcquireError>>,
) -> Result<P, Error> {
    tokio::time::timeout(READ_QUEUE_WAIT, acquire)
        .await
        .map_err(|_| Error::Unavailable)?
        .map_err(|_| Error::Unavailable)
}

pub fn router(state: Arc<State>) -> Router {
    let public = Router::new()
        .route("/health/live", get(|| async { StatusCode::OK }))
        .route("/health/ready", get(ready))
        .route("/v1/traces", post(traces))
        .route("/v1/logs", post(logs))
        .route("/v1/traces/receipt", post(receipt))
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods([http::Method::POST, http::Method::GET])
                .allow_headers([
                    http::header::AUTHORIZATION,
                    http::header::CONTENT_TYPE,
                    http::header::CONTENT_ENCODING,
                ]),
        );
    let routes = public.clone().nest("/lens-ingest", public);
    let routes = if state.service_token.is_some() {
        routes.merge(
            Router::new()
                .route("/internal/read", post(read))
                .route("/internal/spend", post(spend))
                .route("/internal/feedback", post(feedback))
                .route("/internal/status", get(status)),
        )
    } else {
        routes
    };
    routes.with_state(state).merge(lens_server::router())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptRequest {
    trace_id: String,
    #[serde(default)]
    span_ids: Vec<String>,
}

async fn receipt(
    AppState(state): AppState<Arc<State>>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<Value>, Error> {
    let tenant = state.credentials.tenant(&headers)?;
    state.require_storage()?;
    let _permit = wait_for_read_slot(state.read_slots.acquire()).await?;
    let body = tokio::time::timeout(Duration::from_secs(5), to_bytes(body, usize::MAX))
        .await
        .map_err(|_| Error::Unavailable)?
        .map_err(|_| Error::TooLarge)?;
    let request: ReceiptRequest =
        serde_json::from_slice(&body).map_err(|_| Error::InvalidRequest)?;
    let received = litellm_traces_clickhouse::trace_received(
        &state.storage.client,
        state.storage.config.storage().reader(),
        &tenant,
        &request.trace_id,
        &request.span_ids,
    )
    .await?;
    Ok(Json(serde_json::json!({"received": received})))
}

async fn status(
    AppState(state): AppState<Arc<State>>,
    headers: HeaderMap,
) -> Result<Json<Value>, Error> {
    state.authorize_service(&headers)?;
    Ok(Json(serde_json::json!({
        "storage_ready": state.schema_ready.load(Ordering::Acquire),
        "credentials_ready": state.credentials.ready(),
        "release": std::env::var("LENS_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into()),
        "protocol_version": wire::PROTOCOL_VERSION,
        "public_contract": 1,
    })))
}

async fn ready(AppState(state): AppState<Arc<State>>) -> StatusCode {
    if state.schema_ready.load(Ordering::Acquire) && state.credentials.ready() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn traces(
    AppState(state): AppState<Arc<State>>,
    headers: HeaderMap,
    body: Body,
) -> axum::response::Response {
    ingest::receive(state, headers, body, false).await
}

async fn logs(
    AppState(state): AppState<Arc<State>>,
    headers: HeaderMap,
    body: Body,
) -> axum::response::Response {
    ingest::receive(state, headers, body, true).await
}

async fn read(
    AppState(state): AppState<Arc<State>>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<Value>, Error> {
    state.authorize_service(&headers)?;
    state.require_storage()?;
    let permit = wait_for_read_slot(state.read_slots.clone().acquire_owned()).await?;
    let body = tokio::time::timeout(Duration::from_secs(10), to_bytes(body, usize::MAX))
        .await
        .map_err(|_| Error::Unavailable)?
        .map_err(|_| Error::TooLarge)?;
    let request = serde_json::from_slice(&body).map_err(|_| Error::InvalidRequest)?;
    tokio::spawn(async move {
        let _permit = permit;
        state.storage.read(request).await.map(Json)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

async fn spend(
    AppState(state): AppState<Arc<State>>,
    headers: HeaderMap,
    body: Body,
) -> Result<StatusCode, Error> {
    insert(state, headers, body, InsertTable::SpendLogs).await
}

async fn feedback(
    AppState(state): AppState<Arc<State>>,
    headers: HeaderMap,
    body: Body,
) -> Result<StatusCode, Error> {
    insert(state, headers, body, InsertTable::LensFeedback).await
}

async fn insert(
    state: Arc<State>,
    headers: HeaderMap,
    body: Body,
    table: InsertTable,
) -> Result<StatusCode, Error> {
    state.authorize_service(&headers)?;
    state.require_storage()?;
    let permit = state
        .export_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::Unavailable)?;
    let body = tokio::time::timeout(Duration::from_secs(10), to_bytes(body, usize::MAX))
        .await
        .map_err(|_| Error::Unavailable)?
        .map_err(|_| Error::TooLarge)?;
    tokio::spawn(async move {
        let _permit = permit;
        let rows: Vec<BTreeMap<String, Value>> =
            serde_json::from_slice(&body).map_err(|_| Error::InvalidRequest)?;
        litellm_traces_clickhouse::insert_rows(
            &state.storage.client,
            state.storage.config.storage().writer(),
            state.storage.config.storage().database(),
            table,
            rows,
        )
        .await?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

pub async fn provision(state: Arc<State>) {
    loop {
        let ready = if state.schema_ready.load(Ordering::Acquire) {
            tokio::time::timeout(Duration::from_secs(5), state.storage.ping())
                .await
                .is_ok_and(|r| r.is_ok())
        } else {
            tokio::time::timeout(Duration::from_secs(30), state.storage.ensure_schema())
                .await
                .is_ok_and(|r| r.is_ok())
        };
        state.schema_ready.store(ready, Ordering::Release);
        if !ready {
            tracing::warn!("Lens storage unavailable; retrying");
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, READ_QUEUE_WAIT, wait_for_read_slot};
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    #[tokio::test]
    async fn ninth_read_waits_for_a_permit_and_succeeds() {
        let slots = Arc::new(Semaphore::new(8));
        let permits = (0..8)
            .map(|_| slots.clone().try_acquire_owned().expect("available permit"))
            .collect::<Vec<_>>();
        let waiting_slots = slots.clone();
        let waiting =
            tokio::spawn(async move { wait_for_read_slot(waiting_slots.acquire_owned()).await });

        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(permits);
        assert!(waiting.await.expect("joined read").is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn read_queue_timeout_returns_unavailable() {
        let slots = Arc::new(Semaphore::new(0));
        let waiting = tokio::spawn(wait_for_read_slot(slots.acquire_owned()));

        tokio::task::yield_now().await;
        tokio::time::advance(READ_QUEUE_WAIT).await;
        assert!(matches!(
            waiting.await.expect("joined read"),
            Err(Error::Unavailable)
        ));
    }
}
