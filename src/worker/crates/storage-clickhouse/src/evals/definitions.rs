use chrono::{DateTime, SecondsFormat, Utc};
use lens_contract::eval::{EvalDefinition, EvalSpec};

use super::{
    ATTEMPTS, EvalStore, PAGE_SIZE,
    records::{decode, digest, encode, retry},
};
use crate::{Error, EvalError, state::Change};

fn prefix(team: &str) -> String {
    format!("eval-def/{}/", digest(team.as_bytes()))
}

impl EvalStore {
    pub async fn put_definition(
        &self,
        team: &str,
        name: &str,
        spec: EvalSpec,
        now: DateTime<Utc>,
    ) -> Result<EvalDefinition, EvalError> {
        if team.is_empty() {
            return Err(EvalError::InvalidRequest("team is required"));
        }
        let key = format!("{}{name}", prefix(team));
        let definition = EvalDefinition {
            name: name.to_owned(),
            spec,
            updated_at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
        };
        for attempt in 0..ATTEMPTS {
            let previous = self.state.read(&key).await?;
            if !previous.value.is_null() {
                let existing: EvalDefinition = decode(&previous)?;
                if existing.spec == definition.spec {
                    return Ok(existing);
                }
            }
            let change = Change {
                previous,
                value: encode(&definition)?,
            };
            match self.state.commit(vec![change]).await {
                Ok(()) => return Ok(definition),
                Err(Error::StateConflict) => retry(attempt).await,
                Err(error) => return Err(error.into()),
            }
        }
        Err(Error::StateConflict.into())
    }

    pub async fn definition(&self, team: &str, name: &str) -> Result<EvalDefinition, EvalError> {
        let snapshot = self.state.read(&format!("{}{name}", prefix(team))).await?;
        if snapshot.value.is_null() {
            return Err(EvalError::EvalNotFound);
        }
        decode(&snapshot)
    }

    pub async fn definitions(&self, team: &str) -> Result<Vec<EvalDefinition>, EvalError> {
        let prefix = prefix(team);
        let mut after = String::new();
        let mut definitions = Vec::new();
        loop {
            let keys = self.state.keys(&prefix, &after, PAGE_SIZE).await?;
            let Some(last) = keys.last() else {
                return Ok(definitions);
            };
            let snapshots = self
                .state
                .read_many(&keys.iter().map(String::as_str).collect::<Vec<_>>())
                .await?;
            definitions.extend(
                snapshots
                    .iter()
                    .filter(|snapshot| !snapshot.value.is_null())
                    .map(decode::<EvalDefinition>)
                    .collect::<Result<Vec<_>, _>>()?,
            );
            after = last.clone();
        }
    }
}
