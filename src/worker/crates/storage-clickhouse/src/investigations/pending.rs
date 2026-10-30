use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Instant,
};

use chrono::{DateTime, Utc};
use lens_contract::{
    investigations::Lens,
    worker::{Job, Progress},
};
use lens_investigations::{CheckpointError, RepositoryError};
use serde_json::Value;
use tokio::{sync::oneshot, time::Instant as Deadline};

pub(super) type Saved = Result<Option<Lens>, CheckpointError>;

pub(super) struct Write {
    pub(super) assigned: Job,
    pub(super) body: Progress,
    pub(super) now: DateTime<Utc>,
    pub(super) started: Instant,
    pub(super) deadline: Deadline,
    pub(super) checkpoint: Option<(String, Value)>,
}

pub(super) struct Queued {
    ticket: u64,
    pub(super) write: Write,
    pub(super) unconfirmed: bool,
    reply: oneshot::Sender<Saved>,
}

impl Queued {
    pub(super) fn settle(self, saved: Saved) {
        let _ = self.reply.send(saved);
    }

    pub(super) fn failure(&self) -> RepositoryError {
        if self.unconfirmed {
            RepositoryError::WriteUnconfirmed
        } else {
            RepositoryError::Conflict
        }
    }

    pub(super) fn retried(self, unconfirmed: bool) -> Self {
        Self {
            unconfirmed,
            ..self
        }
    }
}

#[derive(Default)]
struct Queues {
    next: u64,
    lenses: HashMap<String, BTreeMap<u64, Queued>>,
}

#[derive(Clone, Default)]
pub(crate) struct Pending(Arc<Mutex<Queues>>);

impl Pending {
    fn queues(&self) -> MutexGuard<'_, Queues> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn queue(&self, lens_id: &str, write: Write) -> (u64, oneshot::Receiver<Saved>) {
        let (reply, saved) = oneshot::channel();
        let mut queues = self.queues();
        queues.lenses.retain(|_, writes| {
            writes.retain(|_, queued| !queued.reply.is_closed());
            !writes.is_empty()
        });
        let ticket = queues.next;
        queues.next += 1;
        queues.lenses.entry(lens_id.to_owned()).or_default().insert(
            ticket,
            Queued {
                ticket,
                write,
                unconfirmed: false,
                reply,
            },
        );
        (ticket, saved)
    }

    pub(super) fn withdraw(&self, lens_id: &str, ticket: u64) -> Option<Queued> {
        let mut queues = self.queues();
        let writes = queues.lenses.get_mut(lens_id)?;
        let queued = writes.remove(&ticket);
        if writes.is_empty() {
            queues.lenses.remove(lens_id);
        }
        queued
    }

    pub(super) fn claim(&self, lens_id: &str) -> Vec<Queued> {
        let claimed = self.queues().lenses.remove(lens_id);
        let now = Deadline::now();
        let (expired, live): (Vec<_>, Vec<_>) = claimed
            .into_iter()
            .flat_map(BTreeMap::into_values)
            .filter(|queued| !queued.reply.is_closed())
            .partition(|queued| queued.write.deadline <= now);
        expired.into_iter().for_each(|queued| {
            let failure = queued.failure();
            queued.settle(Err(failure.into()));
        });
        live
    }

    pub(super) fn restore(&self, lens_id: &str, writes: Vec<Queued>) {
        let live: BTreeMap<_, _> = writes
            .into_iter()
            .filter(|queued| !queued.reply.is_closed())
            .map(|queued| (queued.ticket, queued))
            .collect();
        if live.is_empty() {
            return;
        }
        self.queues()
            .lenses
            .entry(lens_id.to_owned())
            .or_default()
            .extend(live);
    }
}
