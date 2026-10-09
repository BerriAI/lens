use std::{collections::BTreeSet, time::Duration};

use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    investigations::{Lens, Scope},
    worker::{EvidenceRole, Finding, FindingKind, FindingStatus},
};
use lens_investigations::LensRepository;
use litellm_storage_clickhouse::{
    investigations::Investigations,
    state::{Change, Snapshot},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use crate::Error;

const MAX_ATTEMPTS: u8 = 6;

pub struct SlackConfig {
    webhook: Url,
    public_url: Url,
    scope: Scope,
    agent: String,
}

impl SlackConfig {
    pub(crate) fn read(
        read: &impl Fn(&str) -> Option<String>,
        public_url: &str,
    ) -> Result<Option<Self>, Error> {
        let Some(webhook) = read("LENS_SLACK_WEBHOOK_URL").filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let webhook =
            Url::parse(&webhook).map_err(|_| Error::Configuration("LENS_SLACK_WEBHOOK_URL"))?;
        if !clean_https(&webhook)
            || !matches!(
                webhook.host_str(),
                Some("hooks.slack.com" | "hooks.slack-gov.com")
            )
            || !webhook.path().starts_with("/services/")
        {
            return Err(Error::Configuration("LENS_SLACK_WEBHOOK_URL"));
        }
        let public_url =
            Url::parse(public_url).map_err(|_| Error::Configuration("LENS_PUBLIC_URL"))?;
        if !clean_https(&public_url) {
            return Err(Error::Configuration(
                "Slack requires an HTTPS LENS_PUBLIC_URL",
            ));
        }
        let agent = read("LENS_SLACK_AGENT")
            .filter(|value| !value.trim().is_empty())
            .ok_or(Error::Configuration("LENS_SLACK_AGENT"))?;
        let team = read("LENS_SLACK_TEAM_ID").filter(|value| !value.trim().is_empty());
        let all_teams = match read("LENS_SLACK_ALL_TEAMS").as_deref() {
            None | Some("") | Some("false") => false,
            Some("true") => true,
            _ => return Err(Error::Configuration("LENS_SLACK_ALL_TEAMS")),
        };
        if team.is_some() == all_teams {
            return Err(Error::Configuration(
                "Set LENS_SLACK_TEAM_ID or explicitly set LENS_SLACK_ALL_TEAMS=true",
            ));
        }
        Ok(Some(Self {
            webhook,
            public_url,
            scope: Scope {
                team_id: team.unwrap_or_default(),
                all_teams,
                ..Scope::default()
            },
            agent,
        }))
    }

    fn identity(&self) -> String {
        format!(
            "slack-findings/{:x}",
            Sha256::digest(format!(
                "{}\0{}\0{}\0{}",
                self.webhook, self.scope.team_id, self.scope.all_teams, self.agent
            ))
        )
    }
}

fn clean_https(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.port().is_none()
}

#[derive(Clone, Deserialize, Serialize)]
struct Delivery {
    attempts: u8,
    next_attempt: DateTime<Utc>,
    finished: bool,
    token: String,
}

impl Delivery {
    fn claim(previous: Option<Self>, now: DateTime<Utc>) -> Option<Self> {
        let attempts = match previous {
            Some(previous) if previous.finished || previous.next_attempt > now => return None,
            Some(previous) => previous.attempts,
            None => 0,
        };
        (attempts < MAX_ATTEMPTS).then(|| Self {
            attempts: attempts + 1,
            next_attempt: now + TimeDelta::seconds(60),
            finished: false,
            token: uuid::Uuid::new_v4().to_string(),
        })
    }

    fn complete(self, outcome: DeliveryOutcome, now: DateTime<Utc>) -> Self {
        let retry = match outcome {
            DeliveryOutcome::Retry(seconds) | DeliveryOutcome::RateLimited(seconds)
                if self.attempts < MAX_ATTEMPTS =>
            {
                Some(seconds)
            }
            _ => None,
        };
        Self {
            finished: retry.is_none(),
            next_attempt: now + TimeDelta::seconds(retry.unwrap_or(0)),
            ..self
        }
    }
}

enum DeliveryOutcome {
    Delivered,
    Retry(i64),
    RateLimited(i64),
    Rejected,
}

pub struct Notifications {
    config: SlackConfig,
    repository: Investigations,
    client: reqwest::Client,
}

impl Notifications {
    pub fn new(config: SlackConfig, repository: Investigations) -> Result<Self, Error> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self {
            config,
            repository,
            client,
        })
    }

    pub async fn serve(self) {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if self.tick(Utc::now()).await.is_err() {
                tracing::warn!("Slack findings scan failed; retrying without affecting analysis");
            }
        }
    }

    async fn tick(&self, now: DateTime<Utc>) -> Result<(), Error> {
        let identity = self.config.identity();
        let activation = self
            .repository
            .0
            .read(&format!("{identity}/activation"))
            .await?;
        if activation.value.is_null() {
            self.repository
                .0
                .commit(vec![Change {
                    previous: activation,
                    value: json!(now),
                }])
                .await?;
            tracing::info!(
                "Slack findings notifications activated; existing findings will be skipped"
            );
            return Ok(());
        }
        let activated: DateTime<Utc> = serde_json::from_value(activation.value)?;
        let cooldown = self
            .repository
            .0
            .read(&format!("{identity}/cooldown"))
            .await?;
        if !cooldown.value.is_null()
            && serde_json::from_value::<DateTime<Utc>>(cooldown.value.clone())? > now
        {
            return Ok(());
        }
        let lenses = self.repository.lenses(&self.config.scope).await?;
        let mut attempted = 0;
        for lens in lenses
            .iter()
            .filter(|lens| lens.settings.agent_name == self.config.agent)
        {
            for finding in lens
                .findings
                .iter()
                .filter(|finding| eligible(finding, activated))
            {
                let key = format!(
                    "{identity}/finding/{:x}",
                    Sha256::digest(format!("{}\0{}", lens.id, finding.id))
                );
                let previous = self.repository.0.read(&key).await?;
                let delivery = (!previous.value.is_null())
                    .then(|| serde_json::from_value(previous.value.clone()))
                    .transpose()?;
                let Some(claim) = Delivery::claim(delivery, Utc::now()) else {
                    continue;
                };
                self.repository
                    .0
                    .commit(vec![Change {
                        previous,
                        value: serde_json::to_value(&claim)?,
                    }])
                    .await?;
                let claimed = self.repository.0.read(&key).await?;
                let active: Delivery = serde_json::from_value(claimed.value.clone())?;
                if active.token != claim.token
                    || active.next_attempt <= Utc::now() + TimeDelta::seconds(15)
                {
                    continue;
                }
                let outcome = send(
                    &self.client,
                    &self.config.webhook,
                    &message(&self.config, lens, finding),
                    claim.attempts,
                )
                .await;
                if !matches!(outcome, DeliveryOutcome::Delivered) {
                    tracing::warn!(
                        attempt = claim.attempts,
                        "Slack finding delivery failed; inspect Slack configuration if retries are exhausted"
                    );
                }
                let rate_limited = if let DeliveryOutcome::RateLimited(seconds) = outcome {
                    self.repository
                        .0
                        .commit(vec![Change {
                            previous: cooldown.clone(),
                            value: json!(Utc::now() + TimeDelta::seconds(seconds)),
                        }])
                        .await?;
                    true
                } else {
                    false
                };
                self.save_delivery(claimed, claim.complete(outcome, Utc::now()))
                    .await?;
                if rate_limited {
                    return Ok(());
                }
                attempted += 1;
                if attempted == 5 {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        Ok(())
    }

    async fn save_delivery(&self, previous: Snapshot, delivery: Delivery) -> Result<(), Error> {
        self.repository
            .0
            .commit(vec![Change {
                previous,
                value: serde_json::to_value(delivery)?,
            }])
            .await?;
        Ok(())
    }
}

fn eligible(finding: &Finding, activated: DateTime<Utc>) -> bool {
    finding.first_seen >= activated
        && finding.kind == FindingKind::Issue
        && finding.status == FindingStatus::Open
        && finding
            .evidence
            .iter()
            .any(|evidence| evidence.role == EvidenceRole::Support)
}

fn message(config: &SlackConfig, lens: &Lens, finding: &Finding) -> Value {
    let mut link = config
        .public_url
        .join("/ui/")
        .expect("validated absolute URL");
    link.query_pairs_mut()
        .append_pair("tab", "findings")
        .append_pair("agent", &config.agent)
        .append_pair("issue", &format!("{}:{}", lens.id, finding.id));
    let sessions = finding
        .evidence
        .iter()
        .filter(|evidence| evidence.role == EvidenceRole::Support)
        .map(|evidence| &evidence.execution_id)
        .collect::<BTreeSet<_>>()
        .len();
    let title: String = finding.title.as_str().chars().take(500).collect();
    json!({
        "text": "Lens found an improvement candidate. Review the evidence in Lens.",
        "mrkdwn": false, "unfurl_links": false, "unfurl_media": false,
        "blocks": [
            {"type":"header","text":{"type":"plain_text","text":"Lens improvement candidate"}},
            {"type":"section","text":{"type":"plain_text","text":title}},
            {"type":"section","text":{"type":"plain_text","text":format!("Agent: {}\nSupporting sampled runs: {sessions}\nThis finding has not been validated by a paired eval. No quality or speed improvement has been established.", config.agent.chars().take(200).collect::<String>())}},
            {"type":"actions","elements":[{"type":"button","text":{"type":"plain_text","text":"Review evidence in Lens"},"url":link.as_str()}]}
        ]
    })
}

async fn send(
    client: &reqwest::Client,
    webhook: &Url,
    body: &Value,
    attempt: u8,
) -> DeliveryOutcome {
    let backoff = 30_i64 * 2_i64.pow(u32::from(attempt.saturating_sub(1).min(5)));
    let Ok(mut response) = client.post(webhook.clone()).json(body).send().await else {
        return DeliveryOutcome::Retry(backoff);
    };
    match response.status().as_u16() {
        200 => {
            let mut body = Vec::new();
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) if body.len() + chunk.len() <= 64 => {
                        body.extend_from_slice(&chunk)
                    }
                    Ok(None) if body == b"ok" => return DeliveryOutcome::Delivered,
                    Err(_) => return DeliveryOutcome::Retry(backoff),
                    _ => return DeliveryOutcome::Rejected,
                }
            }
        }
        429 => DeliveryOutcome::RateLimited(
            response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u32>().ok())
                .map(i64::from)
                .unwrap_or(backoff)
                .max(30),
        ),
        500..=599 => DeliveryOutcome::Retry(backoff),
        _ => DeliveryOutcome::Rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[fixture]
    fn config() -> SlackConfig {
        SlackConfig::read(
            &|name| match name {
                "LENS_SLACK_WEBHOOK_URL" => {
                    Some("https://hooks.slack.com/services/test/channel/secret".into())
                }
                "LENS_SLACK_AGENT" => Some("test-agent".into()),
                "LENS_SLACK_TEAM_ID" => Some("team".into()),
                _ => None,
            },
            "https://lens.example.com",
        )
        .unwrap()
        .unwrap()
    }

    #[fixture]
    fn finding() -> Finding {
        serde_json::from_value(json!({
            "id":"finding", "title":"Repeated tool failure", "description":"A candidate", "check_id":"check",
            "first_seen":"2026-01-01T00:00:00Z", "last_seen":"2026-01-01T00:00:00Z", "revision":1,
            "evidence":[{"execution_id":"run", "span_id":"span", "quote":"private evidence"}]
        })).unwrap()
    }

    #[fixture]
    fn lens() -> Lens {
        lens_investigations::create_lens(
            serde_json::from_value(json!({"name":"test", "model":"analysis", "agent_name":"test-agent", "monthly_budget":1, "context":"Find supported failures"})).unwrap(),
            Scope { team_id:"team".into(), ..Default::default() }, Utc::now(), "lens", "job",
        ).unwrap()
    }

    #[rstest]
    #[case::disabled(None, None, None, true)]
    #[case::team(
        Some("https://hooks.slack.com/services/t/c/s"),
        Some("team"),
        None,
        true
    )]
    #[case::global(
        Some("https://hooks.slack.com/services/t/c/s"),
        None,
        Some("true"),
        true
    )]
    #[case::unscoped(Some("https://hooks.slack.com/services/t/c/s"), None, None, false)]
    #[case::ambiguous(
        Some("https://hooks.slack.com/services/t/c/s"),
        Some("team"),
        Some("true"),
        false
    )]
    #[case::lookalike(
        Some("https://hooks.slack.com.attacker.test/services/t/c/s"),
        Some("team"),
        None,
        false
    )]
    #[case::insecure(
        Some("http://hooks.slack.com/services/t/c/s"),
        Some("team"),
        None,
        false
    )]
    #[case::credentials(
        Some("https://user@hooks.slack.com/services/t/c/s"),
        Some("team"),
        None,
        false
    )]
    #[case::query(
        Some("https://hooks.slack.com/services/t/c/s?redirect=evil"),
        Some("team"),
        None,
        false
    )]
    fn rejects_unsafe_or_unscoped_configuration(
        #[case] webhook: Option<&str>,
        #[case] team: Option<&str>,
        #[case] all_teams: Option<&str>,
        #[case] valid: bool,
    ) {
        let result = SlackConfig::read(
            &|name| match name {
                "LENS_SLACK_WEBHOOK_URL" => webhook.map(str::to_owned),
                "LENS_SLACK_TEAM_ID" => team.map(str::to_owned),
                "LENS_SLACK_ALL_TEAMS" => all_teams.map(str::to_owned),
                "LENS_SLACK_AGENT" => Some("agent".into()),
                _ => None,
            },
            "https://lens.example.com",
        );
        assert_eq!(result.is_ok(), valid);
    }

    #[rstest]
    #[case::new("open", "issue", "support", -1, true)]
    #[case::historical("open", "issue", "support", 1, false)]
    #[case::resolved("resolved", "issue", "support", -1, false)]
    #[case::dismissed("dismissed", "issue", "support", -1, false)]
    #[case::pattern("open", "pattern", "support", -1, false)]
    #[case::counterexample("open", "issue", "counterexample", -1, false)]
    fn only_new_supported_open_issues(
        finding: Finding,
        #[case] status: &str,
        #[case] kind: &str,
        #[case] role: &str,
        #[case] activation_offset: i64,
        #[case] selected: bool,
    ) {
        let mut value = serde_json::to_value(&finding).unwrap();
        value["status"] = json!(status);
        value["kind"] = json!(kind);
        value["evidence"][0]["role"] = json!(role);
        let updated = serde_json::from_value(value).unwrap();
        assert_eq!(
            eligible(
                &updated,
                finding.first_seen + TimeDelta::seconds(activation_offset)
            ),
            selected
        );
    }

    #[rstest]
    fn message_preserves_uncertainty_and_keeps_evidence_private(
        config: SlackConfig,
        lens: Lens,
        finding: Finding,
    ) {
        let mut value = serde_json::to_value(finding).unwrap();
        value["title"] = json!("<!channel> <@U123> https://attacker.test");
        let payload = message(&config, &lens, &serde_json::from_value(value).unwrap());
        assert_eq!(payload["blocks"][1]["text"]["type"], "plain_text");
        assert_eq!(
            payload["text"],
            "Lens found an improvement candidate. Review the evidence in Lens."
        );
        assert!(!payload.to_string().contains("private evidence"));
        assert!(
            payload["blocks"][2]["text"]["text"]
                .as_str()
                .unwrap()
                .contains("No quality or speed improvement has been established")
        );
        let url = Url::parse(payload["blocks"][3]["elements"][0]["url"].as_str().unwrap()).unwrap();
        assert_eq!(url.host_str(), config.public_url.host_str());
        assert_eq!(
            url.query_pairs()
                .find(|(name, _)| name == "issue")
                .unwrap()
                .1,
            "lens:finding"
        );
    }

    #[rstest]
    fn delivery_lease_retry_and_terminal_state_survive_serialization() {
        let now = Utc::now();
        let claim = Delivery::claim(None, now).unwrap();
        assert!(Delivery::claim(Some(claim.clone()), now + TimeDelta::seconds(59)).is_none());
        let retry = claim.complete(DeliveryOutcome::Retry(120), now);
        let restored = serde_json::from_value(serde_json::to_value(retry).unwrap()).unwrap();
        assert!(Delivery::claim(Some(restored), now + TimeDelta::seconds(119)).is_none());
        let terminal = Delivery::claim(None, now)
            .unwrap()
            .complete(DeliveryOutcome::Delivered, now);
        assert!(Delivery::claim(Some(terminal), now + TimeDelta::days(1)).is_none());
    }

    #[rstest]
    fn abandoned_attempts_are_bounded() {
        let now = Utc::now();
        let previous = Delivery {
            attempts: MAX_ATTEMPTS,
            next_attempt: now,
            finished: false,
            token: "expired".into(),
        };
        assert!(Delivery::claim(Some(previous), now).is_none());
    }

    #[rstest]
    #[case::accepted(200, "", Some(0))]
    #[case::limited(429, "120", Some(120))]
    #[case::long_limit(429, "7200", Some(7200))]
    #[case::invalid_limit(429, "-1", Some(30))]
    #[case::unavailable(503, "", Some(30))]
    #[case::revoked(403, "", None)]
    #[case::redirect(302, "", None)]
    #[tokio::test]
    async fn delivery_classifies_response_without_exposing_body(
        #[case] status: u16,
        #[case] retry_after: &str,
        #[case] retry: Option<i64>,
    ) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webhook"))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("retry-after", retry_after)
                    .insert_header("location", "https://attacker.test")
                    .set_body_string(if status == 200 {
                        "ok"
                    } else {
                        "private response"
                    }),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let result = send(
            &client,
            &format!("{}/webhook", server.uri()).parse().unwrap(),
            &json!({"text":"candidate"}),
            1,
        )
        .await;
        assert_eq!(
            match result {
                DeliveryOutcome::Delivered => Some(0),
                DeliveryOutcome::Retry(seconds) | DeliveryOutcome::RateLimited(seconds) =>
                    Some(seconds),
                DeliveryOutcome::Rejected => None,
            },
            retry
        );
    }

    #[rstest]
    #[tokio::test]
    async fn reconciliation_persists_activation_and_dedup_and_excludes_other_scopes(
        mut config: SlackConfig,
        mut lens: Lens,
        finding: Finding,
    ) {
        use litellm_storage_clickhouse::state::ClickHouseState;
        use testcontainers_modules::{
            clickhouse::ClickHouse,
            testcontainers::{ImageExt, runners::AsyncRunner},
        };

        let container = ClickHouse::default()
            .with_tag(
                "26.9.6.6@sha256:eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e",
            )
            .with_env_var("CLICKHOUSE_SKIP_USER_SETUP", "1")
            .with_copy_to(
                "/etc/clickhouse-server/config.d/lens-keeper.xml",
                include_bytes!("../../storage-clickhouse/tests/state/fixtures/keeper.xml").to_vec(),
            )
            .start()
            .await
            .unwrap();
        let connection = litellm_storage_clickhouse::Connection::writer(&format!(
            "http://{}:{}",
            container.get_host().await.unwrap(),
            container.get_host_port_ipv4(8123).await.unwrap()
        ))
        .unwrap();
        let state = ClickHouseState::new(crate::config::http_client().unwrap(), connection);
        state.initialize("/slack-findings-test").await.unwrap();
        let repository = Investigations(state);
        repository.initialize().await.unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&server)
            .await;
        config.webhook = server.uri().parse().unwrap();
        lens.findings = vec![finding.clone()];
        repository.create(&lens).await.unwrap();
        let notifications = Notifications::new(config, repository.clone()).unwrap();
        notifications.tick(Utc::now()).await.unwrap();
        notifications.tick(Utc::now()).await.unwrap();
        assert!(server.received_requests().await.unwrap().is_empty());

        let current = Finding {
            id: "new-finding".into(),
            first_seen: Utc::now(),
            ..finding
        };
        repository
            .update_locked(&lens.id, |saved| {
                Ok::<_, Error>(Lens {
                    findings: vec![current.clone()],
                    ..saved.clone()
                })
            })
            .await
            .unwrap();
        let other_team = Lens {
            id: "other-team".into(),
            scope: Scope {
                team_id: "another-team".into(),
                ..Default::default()
            },
            findings: vec![current.clone()],
            ..lens.clone()
        };
        repository.create(&other_team).await.unwrap();
        let other_agent = Lens {
            id: "other-agent".into(),
            settings: lens_contract::worker::LensSettings {
                agent_name: "another-agent".into(),
                ..lens.settings.clone()
            },
            findings: vec![current],
            ..lens
        };
        repository.create(&other_agent).await.unwrap();
        let (first, second) = tokio::join!(
            notifications.tick(Utc::now()),
            notifications.tick(Utc::now())
        );
        assert!(first.is_ok() || second.is_ok());
        let restarted = Notifications::new(notifications.config, repository).unwrap();
        restarted.tick(Utc::now()).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let posted: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(
            posted["blocks"][3]["elements"][0]["url"]
                .as_str()
                .unwrap()
                .contains("new-finding")
        );
    }
}
