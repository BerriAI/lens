use std::time::Duration;

use chrono::{DateTime, Utc};
use lens_contract::investigations::Lens;
use lens_investigations::RepositoryError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Investigations, StoredLens, decode, document, failure, key, workers::backoff};
use crate::{
    Error,
    state::{Change, ClickHouseState, Snapshot},
};

const WRITER_LEASE: Duration = Duration::from_secs(10);
const WRITER_WAIT: Duration = Duration::from_secs(15);
const CLEANUP_WAIT: Duration = Duration::from_secs(2);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    token: String,
    expires_at: DateTime<Utc>,
}

pub(super) struct Writer {
    state: ClickHouseState,
    key: String,
    token: String,
    acquired: Option<Snapshot>,
    armed: bool,
}

impl Writer {
    async fn acquire(&mut self) -> Result<(), RepositoryError> {
        let mut attempt = 0;
        loop {
            let previous = self.state.read(&self.key).await.map_err(failure)?;
            let available = previous.value.is_null()
                || decode::<Owner>(previous.value.clone())?.expires_at <= Utc::now();
            if available {
                let owner = Owner {
                    token: self.token.clone(),
                    expires_at: Utc::now() + WRITER_LEASE,
                };
                match self
                    .state
                    .commit(vec![Change {
                        previous,
                        value: document(&owner)?,
                    }])
                    .await
                {
                    Ok(()) => {
                        let acquired = self.state.read(&self.key).await.map_err(failure)?;
                        if acquired.value.is_null() {
                            return Err(RepositoryError::Conflict);
                        }
                        let current: Owner = decode(acquired.value.clone())?;
                        if current.token != self.token || current.expires_at <= Utc::now() {
                            return Err(RepositoryError::Conflict);
                        }
                        self.acquired = Some(acquired);
                        return Ok(());
                    }
                    Err(Error::StateConflict) => (),
                    Err(error) => return Err(failure(error)),
                }
            }
            backoff(attempt).await;
            attempt += 1;
        }
    }

    pub(super) async fn commit(mut self, changes: Vec<Change>) -> Result<(), RepositoryError> {
        let previous = self.acquired.take().ok_or(RepositoryError::Conflict)?;
        let owner: Owner = decode(previous.value.clone())?;
        let remaining = (owner.expires_at - Utc::now())
            .to_std()
            .map_err(|_| RepositoryError::Conflict)?;
        let changes = changes
            .into_iter()
            .chain(std::iter::once(Change {
                previous,
                value: Value::Null,
            }))
            .collect();
        tokio::time::timeout(remaining, self.state.commit(changes))
            .await
            .map_err(|_| RepositoryError::Conflict)?
            .map_err(failure)?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let state = self.state.clone();
        let key = self.key.clone();
        let token = self.token.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(CLEANUP_WAIT, async {
                let previous = state.read(&key).await.map_err(failure)?;
                if previous.value.is_null()
                    || decode::<Owner>(previous.value.clone())?.token != token
                {
                    return Ok::<_, RepositoryError>(());
                }
                state
                    .commit(vec![Change {
                        previous,
                        value: Value::Null,
                    }])
                    .await
                    .map_err(failure)
            })
            .await;
        });
    }
}

impl Investigations {
    pub(super) async fn writer(&self, id: &str) -> Result<Writer, RepositoryError> {
        let mut writer = Writer {
            state: self.0.clone(),
            key: key("lens-writer", (id,))?,
            token: uuid::Uuid::new_v4().to_string(),
            acquired: None,
            armed: true,
        };
        tokio::time::timeout(WRITER_WAIT, writer.acquire())
            .await
            .map_err(|_| RepositoryError::Conflict)??;
        Ok(writer)
    }

    pub async fn update_locked<E: From<RepositoryError>>(
        &self,
        id: &str,
        transform: impl Fn(&Lens) -> Result<Lens, E>,
    ) -> Result<Option<Lens>, E> {
        let writer = self.writer(id).await?;
        let previous = self.snapshot(id).await?;
        if previous.value.is_null() {
            writer.commit(Vec::new()).await?;
            return Ok(None);
        }
        let current = decode::<StoredLens>(previous.value.clone())?.lens;
        let candidate = transform(&current)?;
        Ok(Some(
            self.publish_lens(writer, previous, &current, &candidate, None)
                .await?,
        ))
    }
}
