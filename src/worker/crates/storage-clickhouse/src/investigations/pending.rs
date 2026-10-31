use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use lens_contract::investigations::Lens;
use lens_investigations::RepositoryError;
use serde_json::Value;
use tokio::time::Instant as Deadline;

pub(super) struct Record {
    pub(super) key: String,
    pub(super) value: Value,
}

pub(super) enum Outcome<'a> {
    Saved(&'a Lens),
    Missing,
    Failed(RepositoryError),
}

impl Outcome<'_> {
    pub(super) fn saved_lens<E: From<RepositoryError>>(self) -> Result<Option<Lens>, E> {
        match self {
            Self::Saved(lens) => Ok(Some(lens.clone())),
            Self::Missing => Ok(None),
            Self::Failed(error) => Err(error.into()),
        }
    }
}

pub(super) trait Edit: Send {
    fn abandoned(&self) -> bool;
    fn record(&self) -> Option<&Record>;
    fn replays_unconfirmed(&self) -> bool;
    fn apply(self: Box<Self>, candidate: Lens) -> (Lens, Option<Box<dyn Edit>>);
    fn replacing(self: Box<Self>, stored: Value) -> Option<Box<dyn Edit>>;
    fn answer(self: Box<Self>, outcome: Outcome<'_>);
}

pub(super) struct Queued {
    ticket: u64,
    deadline: Deadline,
    unconfirmed: bool,
    edit: Box<dyn Edit>,
}

impl Queued {
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

    pub(super) fn record(&self) -> Option<&Record> {
        self.edit.record()
    }

    pub(super) fn replays_unconfirmed(&self) -> bool {
        self.edit.replays_unconfirmed()
    }

    pub(super) fn apply(self, candidate: Lens) -> (Lens, Option<Self>) {
        let (edited, kept) = self.edit.apply(candidate);
        (edited, kept.map(|edit| Self { edit, ..self }))
    }

    pub(super) fn replacing(self, stored: Value) -> Option<Self> {
        let edit = self.edit.replacing(stored)?;
        Some(Self { edit, ..self })
    }

    pub(super) fn answer(self, outcome: Outcome<'_>) {
        self.edit.answer(outcome);
    }
}

fn split_at_repeated_record(
    mut queued: BTreeMap<u64, Queued>,
) -> (BTreeMap<u64, Queued>, BTreeMap<u64, Queued>) {
    let mut keys = HashSet::new();
    let repeated = queued
        .iter()
        .find(|(_, edit)| {
            edit.record()
                .is_some_and(|record| !keys.insert(record.key.as_str()))
        })
        .map(|(ticket, _)| *ticket);
    let later = repeated
        .map(|ticket| queued.split_off(&ticket))
        .unwrap_or_default();
    (queued, later)
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

    pub(super) fn queue(&self, lens_id: &str, deadline: Deadline, edit: Box<dyn Edit>) -> u64 {
        let mut queues = self.queues();
        queues.lenses.retain(|_, edits| {
            edits.retain(|_, queued| !queued.edit.abandoned());
            !edits.is_empty()
        });
        let ticket = queues.next;
        queues.next += 1;
        queues.lenses.entry(lens_id.to_owned()).or_default().insert(
            ticket,
            Queued {
                ticket,
                deadline,
                unconfirmed: false,
                edit,
            },
        );
        ticket
    }

    pub(super) fn withdraw(&self, lens_id: &str, ticket: u64) -> Option<Queued> {
        let mut queues = self.queues();
        let edits = queues.lenses.get_mut(lens_id)?;
        let queued = edits.remove(&ticket);
        if edits.is_empty() {
            queues.lenses.remove(lens_id);
        }
        queued
    }

    fn take_batch(&self, lens_id: &str) -> BTreeMap<u64, Queued> {
        let mut queues = self.queues();
        let Some(edits) = queues.lenses.remove(lens_id) else {
            return BTreeMap::new();
        };
        let (batch, later) = split_at_repeated_record(edits);
        if !later.is_empty() {
            queues.lenses.insert(lens_id.to_owned(), later);
        }
        batch
    }

    pub(super) fn claim(&self, lens_id: &str) -> Vec<Queued> {
        let claimed = self.take_batch(lens_id);
        let now = Deadline::now();
        let (expired, live): (Vec<_>, Vec<_>) = claimed
            .into_values()
            .filter(|queued| !queued.edit.abandoned())
            .partition(|queued| queued.deadline <= now);
        expired.into_iter().for_each(|queued| {
            let failure = queued.failure();
            queued.answer(Outcome::Failed(failure));
        });
        live
    }

    pub(super) fn restore(&self, lens_id: &str, edits: Vec<Queued>) {
        let live: BTreeMap<_, _> = edits
            .into_iter()
            .filter(|queued| !queued.edit.abandoned())
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
