use std::time::Duration;

use lens_auth::{
    IngestionError, StoreError,
    ingestion::{INGESTION_KEY_LIMIT, IngestionRepository},
};
use lens_contract::ingestion::IngestionKey;
use rand::Rng;
use serde_json::Value;

use crate::{
    Error,
    state::{Change, ClickHouseState},
};

const CATALOG: &str = "ingestion-keys/catalog";

#[derive(Clone)]
pub struct IngestionKeys(pub ClickHouseState);

fn failure(error: Error) -> IngestionError {
    match error {
        Error::StateConflict | Error::StateExists => StoreError::Conflict,
        error => StoreError::Unavailable(Box::new(error)),
    }
    .into()
}

fn keys(value: Value) -> Result<Vec<IngestionKey>, IngestionError> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(value).map_err(|error| StoreError::Unavailable(Box::new(error)).into())
}

fn backoff_ceiling(attempt: u64) -> u64 {
    20 * (attempt + 1).min(8)
}

impl IngestionKeys {
    async fn update(
        &self,
        transform: impl Fn(Vec<IngestionKey>) -> Result<Vec<IngestionKey>, IngestionError> + Send,
    ) -> Result<(), IngestionError> {
        for attempt in 0..40 {
            let previous = self.0.read(CATALOG).await.map_err(failure)?;
            let updated = transform(keys(previous.value.clone())?)?;
            let value = serde_json::to_value(updated)
                .map_err(|error| StoreError::Unavailable(Box::new(error)))?;
            if value == previous.value {
                return Ok(());
            }
            match self.0.commit(vec![Change { previous, value }]).await {
                Ok(()) => return Ok(()),
                Err(Error::StateConflict) => (),
                Err(error) => return Err(failure(error)),
            }
            let delay = rand::thread_rng().gen_range(0..=backoff_ceiling(attempt));
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        Err(StoreError::Conflict.into())
    }
}

impl IngestionRepository for IngestionKeys {
    async fn list(&self) -> Result<Vec<IngestionKey>, IngestionError> {
        let stored = self.0.read(CATALOG).await.map_err(failure)?;
        let mut keys = keys(stored.value)?;
        if keys.len() > INGESTION_KEY_LIMIT {
            return Err(IngestionError::CatalogLimit);
        }
        keys.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(keys)
    }

    async fn insert(&self, key: &IngestionKey) -> Result<(), IngestionError> {
        self.update(|mut keys| {
            if keys.iter().any(|existing| existing.id == key.id) {
                return Err(IngestionError::AlreadyExists);
            }
            if keys.len() >= INGESTION_KEY_LIMIT {
                return Err(IngestionError::KeyLimit);
            }
            keys.push(key.clone());
            Ok(keys)
        })
        .await
    }

    async fn revoke(&self, id: &str) -> Result<(), IngestionError> {
        self.update(|keys| Ok(keys.into_iter().filter(|key| key.id != id).collect()))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::backoff_ceiling;
    use rstest::rstest;

    #[rstest]
    #[case::first_retry(0, 20)]
    #[case::second_retry(1, 40)]
    #[case::cap(7, 160)]
    #[case::capped(8, 160)]
    #[case::last_retry(39, 160)]
    fn retries_have_bounded_backoff(#[case] attempt: u64, #[case] ceiling_ms: u64) {
        assert_eq!(backoff_ceiling(attempt), ceiling_ms);
    }
}
