use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use lens_contract::investigations::Lens;
use lens_investigations::RepositoryError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    sync::oneshot,
    time::{Instant, timeout_at},
};

use super::{
    Investigations, StoredLens, decode, document, failure, key,
    pending::{Edit, Outcome, Queued, Record},
    workers::backoff,
};
use crate::{
    Error,
    state::{Change, ClickHouseState, Snapshot},
};

const WRITER_LEASE: Duration = Duration::from_secs(10);
const WRITER_WAIT: Duration = Duration::from_secs(15);
const CLEANUP_WAIT: Duration = Duration::from_secs(2);
const WRITE_RETRY_WINDOW: Duration = Duration::from_secs(30);

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
            .map_err(|_| RepositoryError::WriteUnconfirmed)?
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

fn writer_key(id: &str) -> Result<String, RepositoryError> {
    key("lens-writer", (id,))
}

struct TransformEdit<F, E> {
    transform: F,
    reply: oneshot::Sender<Result<Option<Lens>, E>>,
}

impl<F, E> Edit for TransformEdit<F, E>
where
    F: Fn(&Lens) -> Result<Lens, E> + Send + 'static,
    E: From<RepositoryError> + Send + 'static,
{
    fn abandoned(&self) -> bool {
        self.reply.is_closed()
    }

    fn record(&self) -> Option<&Record> {
        None
    }

    fn replays_unconfirmed(&self) -> bool {
        false
    }

    fn apply(self: Box<Self>, candidate: Lens) -> (Lens, Option<Box<dyn Edit>>) {
        let rejection = match (self.transform)(&candidate) {
            Ok(edited) if edited.id == candidate.id => return (edited, Some(self)),
            Ok(_) => failure(Error::InvalidState).into(),
            Err(error) => error,
        };
        let _ = self.reply.send(Err(rejection));
        (candidate, None)
    }

    fn replacing(self: Box<Self>, _stored: Value) -> Option<Box<dyn Edit>> {
        Some(self)
    }

    fn answer(self: Box<Self>, outcome: Outcome<'_>) {
        let _ = self.reply.send(outcome.saved_lens());
    }
}

fn combine(lens: &Lens, claimed: Vec<Queued>) -> (Lens, Vec<Queued>) {
    let mut candidate = lens.clone();
    let mut included = Vec::new();
    for queued in claimed {
        let (edited, kept) = queued.apply(candidate);
        candidate = edited;
        included.extend(kept);
    }
    (candidate, included)
}

fn staged(queued: Queued, previous: Snapshot) -> Option<(Queued, Change)> {
    let value = queued.record()?.value.clone();
    let queued = if previous.value.is_null() {
        queued
    } else {
        queued.replacing(previous.value.clone())?
    };
    Some((queued, Change { previous, value }))
}

fn retry_after(claimed: Vec<Queued>, error: RepositoryError) -> Vec<Queued> {
    match error {
        RepositoryError::Conflict => claimed
            .into_iter()
            .map(|queued| queued.retried(false))
            .collect(),
        RepositoryError::WriteUnconfirmed => claimed
            .into_iter()
            .filter_map(|queued| {
                if queued.replays_unconfirmed() {
                    return Some(queued.retried(true));
                }
                queued.answer(Outcome::Failed(RepositoryError::WriteUnconfirmed));
                None
            })
            .collect(),
        RepositoryError::Unavailable(source) => {
            let source: Arc<dyn std::error::Error + Send + Sync> = Arc::from(source);
            claimed.into_iter().for_each(|queued| {
                queued.answer(Outcome::Failed(RepositoryError::Unavailable(Box::new(
                    source.clone(),
                ))));
            });
            Vec::new()
        }
    }
}

impl Investigations {
    pub(super) async fn writer(&self, id: &str) -> Result<Writer, RepositoryError> {
        let turn_key = writer_key(id)?;
        let investigations = self.clone();
        let id = id.to_owned();
        let mut acquisition = tokio::task::JoinSet::new();
        acquisition.spawn(async move {
            let _turn = investigations.0.turn(&turn_key).await;
            investigations.lease(&id).await
        });
        acquisition
            .join_next()
            .await
            .ok_or(RepositoryError::Conflict)?
            .map_err(|error| RepositoryError::Unavailable(Box::new(error)))?
    }

    async fn lease(&self, id: &str) -> Result<Writer, RepositoryError> {
        let mut writer = Writer {
            state: self.0.clone(),
            key: writer_key(id)?,
            token: uuid::Uuid::new_v4().to_string(),
            acquired: None,
            armed: true,
        };
        tokio::time::timeout(WRITER_WAIT, writer.acquire())
            .await
            .map_err(|_| RepositoryError::Conflict)??;
        Ok(writer)
    }

    pub async fn update_locked<E>(
        &self,
        id: &str,
        transform: impl Fn(&Lens) -> Result<Lens, E> + Send + 'static,
    ) -> Result<Option<Lens>, E>
    where
        E: From<RepositoryError> + Send + 'static,
    {
        self.publish_edit(id, |reply| Box::new(TransformEdit { transform, reply }))
            .await
    }

    pub(super) async fn publish_edit<T, E: From<RepositoryError>>(
        &self,
        lens_id: &str,
        edit: impl FnOnce(oneshot::Sender<Result<T, E>>) -> Box<dyn Edit>,
    ) -> Result<T, E> {
        let turn_key = writer_key(lens_id)?;
        let deadline = Instant::now() + WRITE_RETRY_WINDOW;
        let (reply, mut answer) = oneshot::channel();
        let pending = self.0.pending();
        let ticket = pending.queue(lens_id, deadline, edit(reply));
        tokio::spawn(self.clone().publish_queued(lens_id.to_owned(), turn_key));
        if let Ok(answered) = timeout_at(deadline, &mut answer).await {
            return answered.unwrap_or_else(|_| Err(RepositoryError::WriteUnconfirmed.into()));
        }
        if let Some(queued) = pending.withdraw(lens_id, ticket) {
            return Err(queued.failure().into());
        }
        answer
            .await
            .unwrap_or_else(|_| Err(RepositoryError::WriteUnconfirmed.into()))
    }

    async fn publish_queued(self, lens_id: String, turn_key: String) {
        let pending = self.0.pending();
        for attempt in 0.. {
            let turn = self.0.turn(&turn_key).await;
            let claimed = pending.claim(&lens_id);
            if claimed.is_empty() {
                return;
            }
            let retry = self.publish_claimed(&lens_id, claimed).await;
            if retry.is_empty() {
                continue;
            }
            pending.restore(&lens_id, retry);
            drop(turn);
            backoff(attempt).await;
        }
    }

    async fn publish_claimed(&self, lens_id: &str, claimed: Vec<Queued>) -> Vec<Queued> {
        let Some(deadline) = claimed.iter().map(Queued::deadline).min() else {
            return Vec::new();
        };
        let opened = timeout_at(deadline, self.open(lens_id))
            .await
            .unwrap_or_else(|_| Err(RepositoryError::Conflict));
        let (writer, previous, current) = match opened {
            Ok(Some(opened)) => opened,
            Ok(None) => {
                claimed
                    .into_iter()
                    .for_each(|queued| queued.answer(Outcome::Missing));
                return Vec::new();
            }
            Err(error) => return retry_after(claimed, error),
        };
        let (candidate, applied) = combine(&current.lens, claimed);
        let keys: Vec<_> = applied
            .iter()
            .filter_map(Queued::record)
            .map(|record| record.key.as_str())
            .collect();
        let stored = match self.0.read_many(&keys).await {
            Ok(stored) => stored,
            Err(error) => return retry_after(applied, failure(error)),
        };
        let (recording, plain): (Vec<_>, Vec<_>) = applied
            .into_iter()
            .partition(|queued| queued.record().is_some());
        let (recorded, records): (Vec<_>, Vec<_>) = recording
            .into_iter()
            .zip(stored)
            .filter_map(|(queued, previous)| staged(queued, previous))
            .unzip();
        let included: Vec<_> = plain.into_iter().chain(recorded).collect();
        match self
            .publish_lens(writer, previous, &current.lens, &candidate, records)
            .await
        {
            Ok(lens) => {
                included
                    .into_iter()
                    .for_each(|queued| queued.answer(Outcome::Saved(&lens)));
                Vec::new()
            }
            Err(error) => retry_after(included, error),
        }
    }

    async fn open(
        &self,
        lens_id: &str,
    ) -> Result<Option<(Writer, Snapshot, StoredLens)>, RepositoryError> {
        let writer = self.lease(lens_id).await?;
        let previous = self.snapshot(lens_id).await?;
        if previous.value.is_null() {
            return Ok(None);
        }
        let current = decode(previous.value.clone())?;
        Ok(Some((writer, previous, current)))
    }
}
