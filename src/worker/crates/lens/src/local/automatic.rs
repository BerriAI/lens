use chrono::{DateTime, Utc};
use lens_contract::{activity::ActivitySelection, investigations::Lens, worker::LensSettings};
use lens_investigations::{LensRepository, RepositoryError, create_lens, scheduled_window};
use sha2::{Digest, Sha256};
use std::{num::NonZeroU64, time::Duration};

use super::LocalControl;
use crate::{Error, SampleRequest};

const FIRST_RUN_TRACES: i64 = 10;

fn uncovered(agent: &str, lenses: &[Lens]) -> bool {
    !lenses
        .iter()
        .any(|lens| lens.settings.agent_name.is_empty() || lens.settings.agent_name == agent)
}

fn initial(lens: &Lens) -> bool {
    lens.id.starts_with("auto-agent-") && lens.jobs.is_empty() && lens.last_scan_at.is_none()
}

fn new_analysis(
    agent: &str,
    model: &str,
    scope: lens_contract::investigations::Scope,
    now: DateTime<Utc>,
) -> Result<Lens, Error> {
    let settings = LensSettings {
        agent_name: agent.into(),
        name: "Automatic analysis".try_into().map_err(|_| Error::InvalidRequest)?,
        model: model.try_into().map_err(|_| Error::InvalidRequest)?,
        context: "Look for failed tasks, incorrect answers, repeated tool errors, and unnecessary work. Use trace evidence to explain the issue and suggest a concrete improvement. Do not report problems without supporting evidence.".into(),
        checks: vec![],
        concurrency: NonZeroU64::new(2).unwrap(),
        enabled: true,
        execution_ids: vec![],
        filters: vec![],
        interval_minutes: NonZeroU64::new(60).unwrap(),
        lookback_hours: NonZeroU64::new(24).unwrap(),
        monthly_budget: 10.0,
        sample_percent: 100.0,
        sample_size: NonZeroU64::new(10),
        service: String::new(),
        source: Default::default(),
        team_id: String::new(),
    };
    let identity = serde_json::to_vec(&(&scope, agent))?;
    let id = format!("auto-agent-{:x}", Sha256::digest(identity));
    let lens = create_lens(settings, scope, now, &id, "initial")?;
    Ok(Lens {
        jobs: vec![],
        ..lens
    })
}

impl LocalControl {
    pub async fn reconcile_automatic(&self) -> Result<(), Error> {
        let worker = self.worker().await?;
        if worker.analysis_key_id.is_none() {
            return Ok(());
        }
        let models = self.models.get().models();
        let Some(model) = models.first() else {
            return Ok(());
        };
        let lenses = self.repository.lenses(&worker.scope).await?;
        for agent in self.sources.agents(&worker.scope).await? {
            if !uncovered(&agent, &lenses) {
                continue;
            }
            let lens = new_analysis(&agent, model, worker.scope.clone(), Utc::now())?;
            match self.repository.create(&lens).await {
                Ok(_) | Err(RepositoryError::Conflict) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    pub(super) async fn automatic_ready(
        &self,
        lens: &Lens,
        now: DateTime<Utc>,
    ) -> Result<bool, Error> {
        if !initial(lens) {
            return Ok(true);
        }
        let (start, end) = scheduled_window(lens, now)?;
        let selection = ActivitySelection::from(&lens.settings);
        let sample = self
            .sources
            .sample(
                &lens.scope,
                SampleRequest {
                    selection: &selection,
                    start: start
                        .timestamp_millis()
                        .try_into()
                        .map_err(|_| Error::InvalidRequest)?,
                    end: end
                        .timestamp_millis()
                        .try_into()
                        .map_err(|_| Error::InvalidRequest)?,
                    offset: 0,
                    page_size: 1,
                    preview: true,
                    cursor: "",
                },
            )
            .await?;
        Ok(sample.eligible >= FIRST_RUN_TRACES)
    }

    pub(super) async fn automatic_loop(&self) {
        loop {
            if self.reconcile_automatic().await.is_err() {
                tracing::warn!("Lens could not configure automatic analysis; retrying");
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_contract::investigations::Scope;
    use lens_investigations::{queue_job, update_settings};

    #[rstest::rstest]
    fn automatic_policy_is_stable_waits_and_preserves_disabled_settings() {
        let now = Utc::now();
        let lens = new_analysis("agent", "gateway-alias", Scope::default(), now).unwrap();
        assert!(initial(&lens));
        assert!(lens.settings.enabled);
        assert_eq!(lens.settings.model.as_str(), "gateway-alias");
        assert_eq!(lens.settings.sample_size.unwrap().get(), 10);
        assert_eq!(
            lens.id,
            new_analysis("agent", "another-alias", Scope::default(), now)
                .unwrap()
                .id
        );
        assert!(uncovered("another-agent", std::slice::from_ref(&lens)));
        let disabled = update_settings(
            &lens,
            LensSettings {
                enabled: false,
                ..lens.settings.clone()
            },
            now,
        )
        .unwrap();
        assert!(!uncovered("agent", &[disabled]));
        let scheduled = queue_job(&lens, now, "job", Default::default()).unwrap();
        assert!(!initial(&scheduled));
        let global = Lens {
            settings: LensSettings {
                agent_name: String::new(),
                enabled: false,
                ..lens.settings.clone()
            },
            ..lens
        };
        assert!(!uncovered("another-agent", &[global]));
    }
}
