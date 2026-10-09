use litellm_storage_clickhouse::{
    investigations::Investigations,
    state::{Change, ClickHouseState},
};
use serde::{Deserialize, Serialize};

use crate::{Error, Plan, Report};

const CHECKPOINT: &str = "migration/postgres";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    report: Report,
    complete: bool,
}

async fn reject_existing(state: &ClickHouseState, plan: Option<&Plan>) -> Result<(), Error> {
    let mut after = String::new();
    loop {
        let keys = state.keys("", &after, 128).await?;
        let Some(last) = keys.last() else {
            return Ok(());
        };
        let unknown = keys
            .iter()
            .filter(|key| {
                plan.is_none_or(|plan| {
                    key.as_str() != CHECKPOINT && !plan.records().contains_key(*key)
                })
            })
            .map(String::as_str)
            .collect::<Vec<_>>();
        if state
            .heads(&unknown)
            .await?
            .iter()
            .any(|head| head.revision > 0)
        {
            return Err(Error::TargetConflict);
        }
        after = last.clone();
    }
}

pub async fn import(state: &ClickHouseState, plan: &Plan, keeper_path: &str) -> Result<(), Error> {
    state.initialize(keeper_path).await?;
    Investigations(state.clone()).initialize().await?;
    let checkpoint = state.read(CHECKPOINT).await?;
    if checkpoint.head.revision == 0 {
        reject_existing(state, None).await?;
        state
            .commit(vec![Change {
                previous: checkpoint,
                value: serde_json::to_value(Checkpoint {
                    report: plan.report().clone(),
                    complete: false,
                })?,
            }])
            .await?;
    } else {
        let saved: Checkpoint = serde_json::from_value(checkpoint.value)?;
        if saved.report != *plan.report() {
            return Err(Error::TargetConflict);
        }
    }
    reject_existing(state, Some(plan)).await?;
    let entries = plan.records().iter().collect::<Vec<_>>();
    for chunk in entries.chunks(128) {
        let keys = chunk
            .iter()
            .map(|(key, _)| key.as_str())
            .collect::<Vec<_>>();
        let existing = state.read_many(&keys).await?;
        let mut changes = Vec::new();
        for ((_, value), previous) in chunk.iter().zip(existing) {
            if previous.head.revision == 0 {
                changes.push(Change {
                    previous,
                    value: (*value).clone(),
                });
            } else if previous.value != **value {
                return Err(Error::TargetConflict);
            }
        }
        if !changes.is_empty() {
            state.commit(changes).await?;
        }
        let verified = state.read_many(&keys).await?;
        if chunk
            .iter()
            .zip(verified)
            .any(|((_, expected), actual)| **expected != actual.value)
        {
            return Err(Error::Verification);
        }
    }
    let checkpoint = state.read(CHECKPOINT).await?;
    let saved: Checkpoint = serde_json::from_value(checkpoint.value.clone())?;
    if saved.report != *plan.report() {
        return Err(Error::TargetConflict);
    }
    if !saved.complete {
        state
            .commit(vec![Change {
                previous: checkpoint,
                value: serde_json::to_value(Checkpoint {
                    report: plan.report().clone(),
                    complete: true,
                })?,
            }])
            .await?;
    }
    Ok(())
}
