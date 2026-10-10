use crate::{Error, wire};
use futures_util::future::BoxFuture;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

pub trait JobBackend: Send + Sync {
    fn sample(&self) -> BoxFuture<'_, Result<wire::Sample, Error>>;
    fn reviews(&self) -> BoxFuture<'_, Result<Vec<wire::Review>, Error>>;
    fn content<'a>(
        &'a self,
        execution_id: &'a str,
        cursor: &'a str,
        offset: usize,
    ) -> BoxFuture<'a, Result<wire::ExecutionContent, Error>>;
    fn model<'a>(
        &'a self,
        request: &'a wire::ModelRequest,
    ) -> BoxFuture<'a, Result<wire::ModelResult, Error>>;
    fn progress<'a>(&'a self, progress: &'a wire::Progress) -> BoxFuture<'a, Result<(), Error>>;
    fn heartbeat(&self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move { self.progress(&wire::Progress::default()).await })
    }
    fn finish<'a>(&'a self, result: &'a wire::Result) -> BoxFuture<'a, Result<(), Error>>;
}

#[derive(Clone)]
pub struct JobClient {
    backend: Arc<dyn JobBackend>,
    model_slots: Arc<Semaphore>,
    global_slots: Arc<Semaphore>,
}

impl JobClient {
    pub fn local(
        backend: Arc<dyn JobBackend>,
        concurrency: usize,
        global_slots: Arc<Semaphore>,
    ) -> Self {
        Self {
            backend,
            model_slots: Arc::new(Semaphore::new(concurrency.clamp(1, 16))),
            global_slots,
        }
    }

    pub async fn sample(&self) -> Result<wire::Sample, Error> {
        self.backend.sample().await
    }

    pub async fn reviews(&self) -> Result<Vec<wire::Review>, Error> {
        self.backend.reviews().await
    }

    pub async fn finish(&self, result: &wire::Result) -> Result<(), Error> {
        self.backend.finish(result).await
    }

    pub async fn heartbeat(&self) -> Result<(), Error> {
        self.backend.heartbeat().await
    }

    pub async fn progress(&self, progress: &wire::Progress) -> Result<(), Error> {
        self.backend.progress(progress).await
    }

    pub async fn content(
        &self,
        execution_id: &str,
        cursor: &str,
        offset: usize,
    ) -> Result<wire::ExecutionContent, Error> {
        self.backend.content(execution_id, cursor, offset).await
    }

    pub async fn model(&self, body: &wire::ModelRequest) -> Result<wire::ModelResult, Error> {
        let _permit = self
            .model_slots
            .acquire()
            .await
            .map_err(|_| Error::Unavailable)?;
        let _global_permit = self
            .global_slots
            .acquire()
            .await
            .map_err(|_| Error::Unavailable)?;
        for attempt in 0..=4 {
            match self.backend.model(body).await {
                Err(ref error) if error.retryable() && attempt < 4 => {
                    let requested = match error {
                        Error::Control { retry_after, .. } => retry_after.unwrap_or_default(),
                        _ => 0,
                    };
                    tokio::time::sleep(Duration::from_secs(requested.max(1 << attempt).min(60)))
                        .await;
                }
                result => return result,
            }
        }
        Err(Error::Unavailable)
    }
}
