use chrono::{DateTime, Utc};
use lens_contract::investigations::{InvestigatorState, InvestigatorStatus, Lens};
use litellm_storage_clickhouse::state::ClickHouseState;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub(super) struct Investigator {
    lens_id: String,
    agent: String,
    model: String,
    key: String,
    run_limit: u64,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    #[default]
    Starting,
    Running,
    NoTraces,
    Unchanged,
    Quiet,
    Duplicate,
    Delivered,
    Failed,
    DeliveryUnknown,
    RunLimit,
}

#[derive(Deserialize)]
struct Completed {
    at: i64,
}

#[derive(Default, Deserialize)]
struct Stored {
    day: String,
    next_at: i64,
    lease_until: i64,
    runs: u64,
    posts: u64,
    last_run: Option<Completed>,
    #[serde(default)]
    last_check_at: i64,
    #[serde(default)]
    last_outcome: Outcome,
}

impl super::LocalControl {
    pub fn with_investigator_environment(mut self) -> Self {
        self.investigator = Investigator::from_lookup(|key| std::env::var(key).ok());
        self
    }
}

impl Investigator {
    pub(super) fn manages(&self, lens: &Lens) -> bool {
        lens.id == self.lens_id && lens.settings.agent_name == self.agent
    }
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        if lookup("LENS_INVESTIGATOR_ENABLED")?.trim() != "true"
            || lookup("LENS_SLACK_CHAT_ENABLED")?.trim() != "true"
        {
            return None;
        }
        let agent = lookup("LENS_SLACK_AGENT")?.trim().to_owned();
        let scope = [
            lookup("LENS_SLACK_WORKSPACE_ID")?.trim().to_owned(),
            lookup("LENS_SLACK_CHANNEL_ID")?.trim().to_owned(),
            agent.clone(),
            lookup("LENS_INVESTIGATOR_REPOSITORY")?.trim().to_owned(),
        ];
        let key = format!(
            "investigator/{:x}",
            Sha256::digest(serde_json::to_vec(&scope).ok()?)
        );
        Some(Self {
            lens_id: lookup("LENS_FINDINGS_LENS_ID")?.trim().to_owned(),
            agent,
            model: lookup("LENS_INVESTIGATOR_MODEL")
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "gpt-6-astra".into()),
            key,
            run_limit: lookup("LENS_INVESTIGATOR_DAILY_RUN_LIMIT")
                .and_then(|value| value.parse().ok())
                .unwrap_or(144),
        })
    }

    pub(super) async fn status(
        &self,
        store: &ClickHouseState,
        lenses: &[Lens],
    ) -> Option<InvestigatorStatus> {
        if !lenses.iter().any(|lens| self.manages(lens)) {
            return None;
        }
        let stored = store.read(&self.key).await.ok().and_then(|snapshot| {
            if snapshot.value.is_null() {
                Some(Stored::default())
            } else {
                serde_json::from_value(snapshot.value).ok()
            }
        });
        Some(self.project(stored, Utc::now()))
    }

    fn project(&self, stored: Option<Stored>, now: DateTime<Utc>) -> InvestigatorStatus {
        let available = stored.is_some();
        let value = stored.unwrap_or_default();
        let today = value.day == now.format("%Y-%m-%d").to_string();
        let runs = if today { value.runs } else { 0 };
        let posts = if today { value.posts } else { 0 };
        let (state, detail) = if !available {
            (
                InvestigatorState::Unavailable,
                "Investigator status could not be read. Retrying automatically.",
            )
        } else if runs >= self.run_limit {
            (
                InvestigatorState::DailyLimit,
                "Daily analysis budget reached. Resumes at UTC midnight.",
            )
        } else if value.next_at == 0 {
            (
                InvestigatorState::Starting,
                "The investigator has not checked in yet.",
            )
        } else if now.timestamp_millis()
            > value.next_at.max(value.lease_until).saturating_add(120_000)
        {
            (
                InvestigatorState::Stalled,
                "Investigator check is overdue. The worker has not confirmed progress.",
            )
        } else if value.lease_until > now.timestamp_millis() {
            (
                InvestigatorState::Running,
                "Reviewing traces and checking evidence.",
            )
        } else {
            match value.last_outcome {
                Outcome::Failed | Outcome::Running => (
                    InvestigatorState::Failed,
                    "Last analysis did not finish. Evidence remains eligible for retry.",
                ),
                Outcome::DeliveryUnknown => (
                    InvestigatorState::Failed,
                    "Finding saved, but Slack delivery is unconfirmed. Check the channel before resending.",
                ),
                Outcome::NoTraces => (InvestigatorState::Scheduled, "No eligible traces yet."),
                Outcome::Unchanged => (
                    InvestigatorState::Scheduled,
                    "No changed traces since the previous analysis.",
                ),
                Outcome::Quiet => (
                    InvestigatorState::Scheduled,
                    "Analysis completed. No new finding met the evidence threshold.",
                ),
                Outcome::Duplicate => (
                    InvestigatorState::Scheduled,
                    "Existing finding updated. Duplicate Slack notification suppressed.",
                ),
                Outcome::Delivered => (
                    InvestigatorState::Scheduled,
                    "Finding saved and posted to Slack.",
                ),
                Outcome::Starting | Outcome::RunLimit => (
                    InvestigatorState::Scheduled,
                    "Checks run every 10 minutes. Only supported new findings are posted.",
                ),
            }
        };
        let timestamp = |at| {
            if at > 0 {
                DateTime::from_timestamp_millis(at)
            } else {
                None
            }
        };
        InvestigatorStatus {
            lens_id: self.lens_id.clone(),
            agent_name: self.agent.clone(),
            model: self.model.clone(),
            state,
            detail: detail.into(),
            next_run_at: timestamp(value.next_at),
            last_checked_at: timestamp(value.last_check_at),
            last_analysis_at: value.last_run.and_then(|run| timestamp(run.at)),
            runs_today: runs,
            run_limit: self.run_limit,
            posts_today: posts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    #[fixture]
    fn investigator() -> Investigator {
        Investigator {
            lens_id: "lens".into(),
            agent: "agent".into(),
            model: "model".into(),
            key: "key".into(),
            run_limit: 144,
        }
    }
    #[fixture]
    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    #[rstest]
    #[case::quiet(Outcome::Quiet, "scheduled", "No new finding")]
    #[case::failure(Outcome::Failed, "failed", "did not finish")]
    #[case::unconfirmed(Outcome::DeliveryUnknown, "failed", "delivery is unconfirmed")]
    #[case::posted(Outcome::Delivered, "scheduled", "posted to Slack")]
    fn completed_check_explains_its_outcome(
        investigator: Investigator,
        now: DateTime<Utc>,
        #[case] outcome: Outcome,
        #[case] expected: &str,
        #[case] detail: &str,
    ) {
        let status = investigator.project(
            Some(Stored {
                day: now.format("%Y-%m-%d").to_string(),
                next_at: now.timestamp_millis() + 600_000,
                posts: 3,
                last_outcome: outcome,
                ..Default::default()
            }),
            now,
        );
        assert_eq!(serde_json::to_value(status.state).unwrap(), expected);
        assert!(status.detail.contains(detail));
        assert_eq!(status.posts_today, 3);
    }
    #[rstest]
    fn an_overdue_worker_is_not_reported_as_scheduled(
        investigator: Investigator,
        now: DateTime<Utc>,
    ) {
        let status = investigator.project(
            Some(Stored {
                next_at: now.timestamp_millis() - 180_000,
                ..Default::default()
            }),
            now,
        );
        assert!(matches!(status.state, InvestigatorState::Stalled));
    }
    #[rstest]
    fn storage_failure_is_explicit(investigator: Investigator, now: DateTime<Utc>) {
        assert!(matches!(
            investigator.project(None, now).state,
            InvestigatorState::Unavailable
        ));
    }
    #[rstest]
    fn budget_resets_on_the_same_utc_day_boundary_as_the_agent(
        investigator: Investigator,
        now: DateTime<Utc>,
    ) {
        let previous = Stored {
            day: "2000-01-01".into(),
            next_at: now.timestamp_millis() + 60_000,
            runs: 144,
            posts: 8,
            ..Default::default()
        };
        let status = investigator.project(Some(previous), now);
        assert!(matches!(status.state, InvestigatorState::Scheduled));
        assert_eq!(status.runs_today, 0);
        assert_eq!(status.posts_today, 0);
    }
}
