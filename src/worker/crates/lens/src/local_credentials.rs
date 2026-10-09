use std::{sync::Arc, time::Duration};

use lens_auth::{
    IngestionError,
    ingestion::{Ingestion, IngestionRepository},
};
use tokio::sync::Mutex;

use crate::auth::Credentials;

pub struct LocalCredentials<R> {
    ingestion: Ingestion<R>,
    credentials: Arc<Credentials>,
    refresh: Mutex<()>,
}

impl<R: IngestionRepository> LocalCredentials<R> {
    pub fn new(ingestion: Ingestion<R>, credentials: Arc<Credentials>) -> Self {
        Self {
            ingestion,
            credentials,
            refresh: Mutex::new(()),
        }
    }

    pub async fn synchronize(&self) -> Result<bool, IngestionError> {
        let _refresh = self.refresh.lock().await;
        let snapshot = self.ingestion.current_snapshot(chrono::Utc::now()).await?;
        Ok(self.credentials.replace_local(snapshot).is_ok())
    }

    pub async fn serve(self: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            if !matches!(self.synchronize().await, Ok(true)) {
                tracing::warn!("Lens local ingestion credential refresh failed");
            }
        }
    }
}

impl<R: IngestionRepository> lens_server::ingestion::CredentialPublisher for LocalCredentials<R> {
    async fn refresh(&self) -> Result<bool, IngestionError> {
        self.synchronize().await
    }
}
