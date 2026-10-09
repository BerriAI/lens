use super::Control;
use crate::{Error, wire};
use futures_util::future::BoxFuture;
use http::Method;
use serde::{Serialize, de::DeserializeOwned};
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
    fn finish<'a>(&'a self, result: &'a wire::Result) -> BoxFuture<'a, Result<(), Error>>;
}

#[derive(Clone)]
enum Backend {
    Remote {
        control: Box<Control>,
        prefix: String,
    },
    Local(Arc<dyn JobBackend>),
}

#[derive(Clone)]
pub struct JobClient {
    backend: Backend,
    model_slots: Arc<Semaphore>,
    global_slots: Arc<Semaphore>,
}

impl JobClient {
    pub fn with_attempt(mut self, attempt: u64) -> Self {
        if let Backend::Remote { control, .. } = &mut self.backend {
            control.attempt = Some(attempt);
        }
        self
    }

    pub fn new(
        control: Control,
        lens_id: &str,
        job_id: &str,
        concurrency: usize,
    ) -> Result<Self, Error> {
        if [lens_id, job_id].iter().any(|id| {
            id.is_empty()
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        }) {
            return Err(Error::InvalidRequest);
        }
        let global_slots = control.model_slots.clone();
        Ok(Self {
            backend: Backend::Remote {
                control: Box::new(control),
                prefix: format!("lens/worker/{lens_id}/{job_id}"),
            },
            model_slots: Arc::new(Semaphore::new(concurrency.clamp(1, 16))),
            global_slots,
        })
    }

    pub fn local(
        backend: Arc<dyn JobBackend>,
        concurrency: usize,
        global_slots: Arc<Semaphore>,
    ) -> Self {
        Self {
            backend: Backend::Local(backend),
            model_slots: Arc::new(Semaphore::new(concurrency.clamp(1, 16))),
            global_slots,
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        let Backend::Remote { control, prefix } = &self.backend else {
            return Err(Error::InvalidRequest);
        };
        control.get(&format!("{prefix}/{path}")).await
    }

    pub async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T, Error> {
        let Backend::Remote { control, prefix } = &self.backend else {
            return Err(Error::InvalidRequest);
        };
        control.post(&format!("{prefix}/{path}"), body).await
    }

    pub async fn sample(&self) -> Result<wire::Sample, Error> {
        match &self.backend {
            Backend::Local(backend) => backend.sample().await,
            Backend::Remote { .. } => self.get("sample").await,
        }
    }
    pub async fn reviews(&self) -> Result<Vec<wire::Review>, Error> {
        match &self.backend {
            Backend::Local(backend) => backend.reviews().await,
            Backend::Remote { .. } => self.get("reviews").await,
        }
    }
    pub async fn finish(&self, result: &wire::Result) -> Result<(), Error> {
        match &self.backend {
            Backend::Local(backend) => backend.finish(result).await,
            Backend::Remote { .. } => {
                let _: serde_json::Value = self.post("result", result).await?;
                Ok(())
            }
        }
    }
    pub async fn heartbeat(&self) -> Result<(), Error> {
        match &self.backend {
            Backend::Local(backend) => backend.progress(&wire::Progress::default()).await,
            Backend::Remote { .. } => {
                let _: serde_json::Value = self.post("heartbeat", &serde_json::json!({})).await?;
                Ok(())
            }
        }
    }
    pub async fn progress(&self, progress: &wire::Progress) -> Result<(), Error> {
        match &self.backend {
            Backend::Local(backend) => backend.progress(progress).await,
            Backend::Remote { .. } => {
                let _: serde_json::Value = self.post("progress", progress).await?;
                Ok(())
            }
        }
    }
    pub async fn content(
        &self,
        execution_id: &str,
        cursor: &str,
        offset: usize,
    ) -> Result<wire::ExecutionContent, Error> {
        match &self.backend {
            Backend::Local(backend) => backend.content(execution_id, cursor, offset).await,
            Backend::Remote { control, prefix } => {
                let mut url = control.url(&format!("{prefix}/content"))?;
                url.query_pairs_mut()
                    .append_pair("execution_id", execution_id)
                    .append_pair("cursor", cursor)
                    .append_pair("offset", &offset.to_string());
                control
                    .request(Method::GET, url, None::<&()>, Duration::from_secs(180))
                    .await
            }
        }
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
            let result = match &self.backend {
                Backend::Local(backend) => backend.model(body).await,
                Backend::Remote { control, prefix } => {
                    control
                        .request(
                            Method::POST,
                            control.url(&format!("{prefix}/model"))?,
                            Some(body),
                            Duration::from_secs(1800),
                        )
                        .await
                }
            };
            match result {
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
