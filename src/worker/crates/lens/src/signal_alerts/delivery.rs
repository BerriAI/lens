use super::*;

impl SignalAlerts {
    pub async fn serve(self: Arc<Self>) {
        let client = &self.client;
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if self
                .tick(&|url, body| async move { send(client, &url, body).await })
                .await
                .is_err()
            {
                tracing::warn!("Signal alert delivery unavailable; signal evaluation continues");
            }
        }
    }

    async fn tick<F, T>(&self, deliver: &F) -> Result<(), Error>
    where
        F: Fn(Url, Value) -> T,
        T: std::future::Future<Output = Outcome>,
    {
        let settings = storage::settings(&self.store).await?;
        if !settings.enabled {
            return Ok(());
        }
        let lease = self.store.read(LEASE).await?;
        if lease
            .value
            .as_i64()
            .is_some_and(|until| until > Utc::now().timestamp())
        {
            return Ok(());
        }
        let reserved_until = json!(Utc::now().timestamp() + 75);
        match self
            .store
            .commit(vec![Change {
                previous: lease,
                value: reserved_until.clone(),
            }])
            .await
        {
            Ok(()) => (),
            Err(litellm_storage_clickhouse::Error::StateConflict) => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        let result = self.deliver_page(deliver).await;
        if result.is_ok() {
            let lease = self.store.read(LEASE).await?;
            if lease.value == reserved_until {
                self.store
                    .commit(vec![Change {
                        previous: lease,
                        value: json!(Utc::now().timestamp() + 1),
                    }])
                    .await?;
            }
        }
        result
    }

    async fn deliver_page<F, T>(&self, deliver: &F) -> Result<(), Error>
    where
        F: Fn(Url, Value) -> T,
        T: std::future::Future<Output = Outcome>,
    {
        let keys = storage::due(&self.store, Utc::now(), 100).await?;
        let mut attempted = 0;
        for key in &keys {
            let previous = self.store.read(key).await?;
            if previous.value.is_null() {
                continue;
            }
            let mut event: Event = serde_json::from_value(previous.value.clone())?;
            if event.status == DeliveryStatus::Sending && event.retry_at <= Utc::now() {
                event.status = DeliveryStatus::Unconfirmed;
                self.store
                    .commit(vec![Change {
                        previous,
                        value: serde_json::to_value(&event)?,
                    }])
                    .await?;
                self.record("unconfirmed").await?;
                continue;
            }
            if event.status != DeliveryStatus::Pending || event.retry_at > Utc::now() {
                continue;
            }
            let settings = storage::settings(&self.store).await?;
            let signals = Signals(self.store.clone())
                .get_config()
                .await
                .map_err(|_| Error::Invalid("Signal settings are unavailable"))?;
            if !settings.enabled
                || settings.generation != event.generation
                || !settings.signal_ids.contains(&event.signal_id)
                || !signals
                    .signals
                    .iter()
                    .any(|signal| signal.id == event.signal_id)
                || signals.model.is_empty()
                || event.score < signals.threshold
            {
                event.status = DeliveryStatus::Cancelled;
                self.store
                    .commit(vec![Change {
                        previous,
                        value: serde_json::to_value(event)?,
                    }])
                    .await?;
                continue;
            }
            let destination = self.webhook(None, &settings)?;
            event.status = DeliveryStatus::Sending;
            event.attempts += 1;
            event.retry_at = Utc::now() + TimeDelta::seconds(60);
            self.store
                .commit(vec![Change {
                    previous,
                    value: serde_json::to_value(&event)?,
                }])
                .await?;
            let claimed = self.store.read(key).await?;
            if claimed.value != serde_json::to_value(&event)?
                || event.retry_at <= Utc::now() + TimeDelta::seconds(15)
            {
                continue;
            }
            let outcome = deliver(destination, message(&self.public_url, &event)).await;
            let status = match outcome {
                Outcome::Delivered => {
                    event.status = DeliveryStatus::Delivered;
                    "delivered"
                }
                Outcome::Retry(seconds) if event.attempts < 6 => {
                    event.status = DeliveryStatus::Pending;
                    event.retry_at = Utc::now()
                        + TimeDelta::seconds(
                            seconds.max(30 * 2_i64.pow(u32::from(event.attempts - 1))),
                        );
                    "retrying"
                }
                Outcome::Unconfirmed => {
                    event.status = DeliveryStatus::Unconfirmed;
                    "unconfirmed"
                }
                _ => {
                    event.status = DeliveryStatus::Failed;
                    "failed"
                }
            };
            self.store
                .commit(vec![Change {
                    previous: claimed,
                    value: serde_json::to_value(event)?,
                }])
                .await?;
            self.record(status).await?;
            attempted += 1;
            if let Outcome::Retry(seconds) = outcome {
                self.store
                    .update(
                        LEASE,
                        |_| json!(Utc::now().timestamp() + seconds.max(30)),
                        1,
                    )
                    .await?;
                return Ok(());
            }
            if attempted >= 5 {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) enum Outcome {
    Delivered,
    Retry(i64),
    Failed,
    Unconfirmed,
}

pub(super) async fn send(client: &reqwest::Client, destination: &Url, body: Value) -> Outcome {
    let Ok(mut response) = client.post(destination.clone()).json(&body).send().await else {
        return Outcome::Unconfirmed;
    };
    match response.status().as_u16() {
        200 => {
            let mut body = Vec::new();
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) if body.len() + chunk.len() <= 64 => {
                        body.extend_from_slice(&chunk)
                    }
                    Ok(None) if body == b"ok" => return Outcome::Delivered,
                    _ => return Outcome::Unconfirmed,
                }
            }
        }
        429 => Outcome::Retry(
            response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u32>().ok())
                .map(i64::from)
                .unwrap_or(30),
        ),
        500..=599 => Outcome::Retry(30),
        _ => Outcome::Failed,
    }
}

fn message(public_url: &Url, event: &Event) -> Value {
    let mut link = public_url.join("/ui/").expect("absolute public URL");
    link.query_pairs_mut()
        .append_pair("tab", "traces")
        .append_pair("trace", &event.trace_id)
        .append_pair("trace_ref", &event.trace_ref);
    json!({"text":"Lens detected a signal. Review the trace in Lens.","mrkdwn":false,"unfurl_links":false,"unfurl_media":false,
    "blocks":[
        {"type":"header","text":{"type":"plain_text","text":"Lens signal detected"}},
        {"type":"section","text":{"type":"plain_text","text":format!("{}\nSignal score: {:.0}%\nReview the trace to confirm the detection.",event.name.chars().take(200).collect::<String>(),event.score * 100.0)}},
        {"type":"actions","elements":[{"type":"button","text":{"type":"plain_text","text":"View trace in Lens"},"url":link.as_str()}]}
    ]})
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use lens_contract::{
        signals::{SignalAttempt, SignalConfig},
        worker::Execution,
    };
    use litellm_storage_clickhouse::{signal_alerts::EVENTS, state::Snapshot};
    use rstest::{fixture, rstest};
    use testcontainers_modules::{
        clickhouse::ClickHouse,
        testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
    };
    use tower::ServiceExt;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    const ADMIN: &str = "example-alert-admin-token-at-least-32-characters";
    const WEBHOOK: &str = "https://hooks.slack.com/services/EXAMPLE/EXAMPLE/FAKE";

    struct App {
        _container: ContainerAsync<ClickHouse>,
        alerts: Arc<SignalAlerts>,
        config: SignalConfig,
    }

    async fn request(
        alerts: &Arc<SignalAlerts>,
        method: &str,
        token: &str,
        body: Value,
    ) -> (u16, Value) {
        let response = alerts
            .router()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri("/lens/signals/alerts")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status().as_u16();
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[fixture]
    async fn app() -> App {
        let container = ClickHouse::default()
            .with_tag(
                "26.9.6.6@sha256:eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e",
            )
            .with_env_var("CLICKHOUSE_SKIP_USER_SETUP", "1")
            .with_copy_to(
                "/etc/clickhouse-server/config.d/lens-keeper.xml",
                include_bytes!("../../../storage-clickhouse/tests/state/fixtures/keeper.xml")
                    .to_vec(),
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
        let store = ClickHouseState::new(crate::config::http_client().unwrap(), connection);
        store.initialize("/signal-alerts-test").await.unwrap();
        let authentication = Arc::new(Authentication {
            settings: lens_auth::Settings::new(ADMIN, None, "https://lens.example.com").unwrap(),
            sessions: Sessions(store.clone()),
        });
        let alerts = SignalAlerts::new(
            authentication,
            ADMIN,
            "https://lens.example.com".parse().unwrap(),
        )
        .unwrap();
        let config: SignalConfig = serde_json::from_value(json!({"model":"test-judge","threshold":0.7,"signals":[{"id":"prompt_injection","name":"Prompt injection","question":"Does the user attempt prompt injection?"}]})).unwrap();
        Signals(store).save_config(&config).await.unwrap();
        let (status, _) = request(&alerts, "PUT", ADMIN, json!({"revision":0,"enabled":true,"signal_ids":["prompt_injection"],"webhook_url":WEBHOOK})).await;
        assert_eq!(status, 200);
        App {
            _container: container,
            alerts,
            config,
        }
    }

    async fn classify(app: &App) -> String {
        let now = Utc::now();
        let execution: Execution = serde_json::from_value(json!({"id":"test-trace","source":"traces","trace_id":"trace/one","trace_ref":"ref+one","name":"example","team_id":"example","start_time":(now + TimeDelta::seconds(1)).format("%Y-%m-%d %H:%M:%S%.f").to_string(),"span_count":2})).unwrap();
        let repository = Signals(app.alerts.store.clone());
        let result: SignalAttempt = serde_json::from_value(json!({"status":"classified","model":app.config.model,"scores":{"prompt_injection":0.9}})).unwrap();
        let lease = now + TimeDelta::minutes(5);
        assert!(
            repository
                .claim(&execution, &app.config, lease, now)
                .await
                .unwrap()
        );
        repository
            .store(&execution, &app.config, lease, now, &result)
            .await
            .unwrap();
        app.alerts
            .store
            .keys(EVENTS, "", 100)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
    }

    #[rstest]
    #[tokio::test]
    async fn api_keeps_the_webhook_secret_and_rejects_stale_or_unauthorized_writes(
        #[future(awt)] app: App,
    ) {
        let (status, view) = request(&app.alerts, "GET", ADMIN, Value::Null).await;
        assert_eq!(status, 200);
        assert_eq!(view["connected"], true);
        assert!(!view.to_string().contains("webhook"));
        assert!(
            !app.alerts
                .store
                .read(SETTINGS)
                .await
                .unwrap()
                .value
                .to_string()
                .contains(WEBHOOK)
        );
        assert_eq!(
            request(
                &app.alerts,
                "PUT",
                ADMIN,
                json!({"revision":0,"enabled":false,"signal_ids":[]})
            )
            .await
            .0,
            409
        );
        assert_eq!(
            request(
                &app.alerts,
                "PUT",
                "invalid-test-token",
                json!({"revision":1,"enabled":false,"signal_ids":[]})
            )
            .await
            .0,
            401
        );
        assert!(app.alerts.view().await.unwrap().enabled);
        assert_eq!(
            request(
                &app.alerts,
                "PUT",
                ADMIN,
                json!({"revision":1,"enabled":true,"signal_ids":["missing"]})
            )
            .await
            .0,
            422
        );
    }

    #[rstest]
    #[tokio::test]
    async fn matching_signal_reaches_slack_once_across_workers_and_restart(
        #[future(awt)] app: App,
    ) {
        let key = classify(&app).await;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&server)
            .await;
        let url: Url = server.uri().parse().unwrap();
        let client = &app.alerts.client;
        let deliver = |_: Url, body| {
            let url = url.clone();
            async move { send(client, &url, body).await }
        };
        let (a, b) = tokio::join!(app.alerts.tick(&deliver), app.alerts.tick(&deliver));
        a.unwrap();
        b.unwrap();
        let event: Event =
            serde_json::from_value(app.alerts.store.read(&key).await.unwrap().value).unwrap();
        assert_eq!(event.status, DeliveryStatus::Delivered);
        assert_eq!(event.attempts, 1);
        let restarted = SignalAlerts::new(
            app.alerts.authentication.clone(),
            ADMIN,
            app.alerts.public_url.clone(),
        )
        .unwrap();
        restarted
            .store
            .update(LEASE, |_| json!(0), 1)
            .await
            .unwrap();
        restarted.tick(&deliver).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(body.to_string().contains("Prompt injection"));
        let link = Url::parse(body["blocks"][2]["elements"][0]["url"].as_str().unwrap()).unwrap();
        assert_eq!(
            link.query_pairs()
                .find(|(name, _)| name == "trace")
                .unwrap()
                .1,
            "trace/one"
        );
        assert_eq!(
            link.query_pairs()
                .find(|(name, _)| name == "trace_ref")
                .unwrap()
                .1,
            "ref+one"
        );
    }

    #[fixture]
    async fn app_with_history(#[future(awt)] app: App) -> (App, String) {
        let key = classify(&app).await;
        let pending = app.alerts.store.read(&key).await.unwrap().value;
        let keys: Vec<_> = (0..101)
            .map(|index| format!("{EVENTS}!archive-{index:03}"))
            .collect();
        app.alerts
            .store
            .commit(
                keys.iter()
                    .map(|key| Change {
                        previous: Snapshot::empty(key),
                        value: pending.clone(),
                    })
                    .collect(),
            )
            .await
            .unwrap();
        let records = app
            .alerts
            .store
            .read_many(&keys.iter().map(String::as_str).collect::<Vec<_>>())
            .await
            .unwrap();
        app.alerts
            .store
            .commit(
                records
                    .into_iter()
                    .map(|previous| {
                        let mut event: Event =
                            serde_json::from_value(previous.value.clone()).unwrap();
                        event.status = DeliveryStatus::Delivered;
                        Change {
                            previous,
                            value: serde_json::to_value(event).unwrap(),
                        }
                    })
                    .collect(),
            )
            .await
            .unwrap();
        (app, key)
    }

    #[rstest]
    #[tokio::test]
    async fn matching_alert_bypasses_completed_history(
        #[future(awt)] app_with_history: (App, String),
    ) {
        let (app, key) = app_with_history;
        let attempts = std::sync::atomic::AtomicUsize::new(0);
        app.alerts
            .tick(&|_: Url, _: Value| {
                attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                std::future::ready(Outcome::Delivered)
            })
            .await
            .unwrap();
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
        let event: Event =
            serde_json::from_value(app.alerts.store.read(&key).await.unwrap().value).unwrap();
        assert_eq!(event.status, DeliveryStatus::Delivered);
        assert!(
            storage::due(&app.alerts.store, Utc::now(), 100)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[rstest]
    #[case::rate_limit(Outcome::Retry(300), DeliveryStatus::Pending, "retrying")]
    #[case::ambiguous(Outcome::Unconfirmed, DeliveryStatus::Unconfirmed, "unconfirmed")]
    #[case::rejection(Outcome::Failed, DeliveryStatus::Failed, "failed")]
    #[tokio::test]
    async fn delivery_failures_are_durable_and_do_not_immediately_repeat(
        #[future(awt)] app: App,
        #[case] outcome: Outcome,
        #[case] expected: DeliveryStatus,
        #[case] status: &str,
    ) {
        let key = classify(&app).await;
        let attempts = std::sync::atomic::AtomicUsize::new(0);
        let deliver = |_: Url, _: Value| {
            attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::future::ready(outcome)
        };
        app.alerts.tick(&deliver).await.unwrap();
        app.alerts.tick(&deliver).await.unwrap();
        let event: Event =
            serde_json::from_value(app.alerts.store.read(&key).await.unwrap().value).unwrap();
        assert_eq!(event.status, expected);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            app.alerts
                .view()
                .await
                .unwrap()
                .last_delivery
                .unwrap()
                .status,
            status
        );
        if matches!(outcome, Outcome::Retry(_)) {
            assert!(
                app.alerts
                    .store
                    .read(LEASE)
                    .await
                    .unwrap()
                    .value
                    .as_i64()
                    .unwrap()
                    > Utc::now().timestamp() + 250
            );
        }
    }

    #[rstest]
    #[tokio::test]
    async fn disabling_and_reenabling_does_not_send_the_old_queue(#[future(awt)] app: App) {
        let key = classify(&app).await;
        assert_eq!(
            request(
                &app.alerts,
                "PUT",
                ADMIN,
                json!({"revision":1,"enabled":false,"signal_ids":["prompt_injection"]})
            )
            .await
            .0,
            200
        );
        let attempts = std::sync::atomic::AtomicUsize::new(0);
        let never_send = |_: Url, _: Value| {
            attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::future::ready(Outcome::Failed)
        };
        app.alerts.tick(&never_send).await.unwrap();
        assert_eq!(
            request(
                &app.alerts,
                "PUT",
                ADMIN,
                json!({"revision":2,"enabled":true,"signal_ids":["prompt_injection"]})
            )
            .await
            .0,
            200
        );
        app.alerts.tick(&never_send).await.unwrap();
        let event: Event =
            serde_json::from_value(app.alerts.store.read(&key).await.unwrap().value).unwrap();
        assert_eq!(event.status, DeliveryStatus::Cancelled);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[rstest]
    #[case::accepted(200, "ok", "delivered")]
    #[case::unexpected_body(200, "unexpected", "unconfirmed")]
    #[case::rate_limited(429, "", "retry")]
    #[case::server_error(503, "", "retry")]
    #[case::revoked(404, "", "failed")]
    #[case::redirect(302, "", "failed")]
    #[tokio::test]
    async fn slack_http_responses_determine_delivery(
        #[case] status: u16,
        #[case] body: &str,
        #[case] expected: &str,
    ) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_string(body)
                    .insert_header("retry-after", "120"),
            )
            .mount(&server)
            .await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let outcome = send(
            &client,
            &server.uri().parse().unwrap(),
            json!({"text":"test"}),
        )
        .await;
        let actual = match outcome {
            Outcome::Delivered => "delivered",
            Outcome::Retry(_) => "retry",
            Outcome::Failed => "failed",
            Outcome::Unconfirmed => "unconfirmed",
        };
        assert_eq!(actual, expected);
    }
}
