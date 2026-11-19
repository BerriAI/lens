mod delivery;
mod secret;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method},
    routing::{get, post},
};
use chrono::{TimeDelta, Utc};
use lens_auth::Authentication;
use lens_contract::{
    auth::Role,
    signal_alerts::{SignalAlertDelivery, SignalAlertRequest, SignalAlertTest, SignalAlertView},
};
use lens_signals::SignalRepository;
use litellm_storage_clickhouse::{
    sessions::Sessions,
    signal_alerts::{self as storage, DeliveryStatus, EVENTS, Event, SETTINGS, Settings},
    signals::Signals,
    state::{Change, ClickHouseState},
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use url::Url;

use crate::error::SignalAlertError as Error;

const LAST_DELIVERY: &str = "signal-alerts/last-delivery";
const LEASE: &str = "signal-alerts/delivery-lease";

pub struct SignalAlerts {
    store: ClickHouseState,
    authentication: Arc<Authentication<Sessions>>,
    secret: secret::Secret,
    client: reqwest::Client,
    public_url: Url,
}

impl SignalAlerts {
    pub fn new(
        authentication: Arc<Authentication<Sessions>>,
        seed: &str,
        public_url: Url,
    ) -> Result<Arc<Self>, crate::Error> {
        Ok(Arc::new(Self {
            store: authentication.sessions.0.clone(),
            authentication,
            secret: secret::Secret::new(seed)?,
            public_url,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(10))
                .build()?,
        }))
    }

    pub fn router(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/lens/signals/alerts", get(read).put(save))
            .route("/lens/signals/alerts/test", post(test))
            .layer(DefaultBodyLimit::max(16 * 1024))
            .with_state(self.clone())
    }

    async fn authorize(&self, headers: &HeaderMap, method: &Method) -> Result<(), Error> {
        let identity = lens_server::auth::identity(&self.authentication, headers, method).await?;
        if identity.user_role != Role::ProxyAdmin {
            return Err(Error::Forbidden);
        }
        Ok(())
    }

    async fn view(&self) -> Result<SignalAlertView, Error> {
        let snapshot = self.store.read(SETTINGS).await?;
        let settings: Settings = if snapshot.value.is_null() {
            Settings::default()
        } else {
            serde_json::from_value(snapshot.value)?
        };
        let delivery = self.store.read(LAST_DELIVERY).await?.value;
        Ok(SignalAlertView {
            revision: snapshot.head.revision,
            enabled: settings.enabled,
            signal_ids: settings.signal_ids,
            connected: !settings.webhook.is_null(),
            last_delivery: if delivery.is_null() {
                None
            } else {
                Some(serde_json::from_value(delivery)?)
            },
        })
    }

    fn webhook(&self, supplied: Option<&str>, settings: &Settings) -> Result<Url, Error> {
        match supplied.filter(|value| !value.trim().is_empty()) {
            Some(value) => webhook(value.trim()),
            None if settings.webhook.is_null() => {
                Err(Error::Invalid("Add a Slack incoming webhook URL"))
            }
            None => webhook(&self.secret.decrypt(&settings.webhook)?),
        }
    }

    async fn record(&self, status: &str) -> Result<(), Error> {
        let value = serde_json::to_value(SignalAlertDelivery {
            at: Utc::now(),
            status: status.into(),
        })?;
        self.store
            .update(LAST_DELIVERY, |_| value.clone(), 8)
            .await?;
        Ok(())
    }
}

fn webhook(value: &str) -> Result<Url, Error> {
    let invalid = || {
        Error::Invalid(
            "Use an HTTPS Slack incoming webhook URL from hooks.slack.com or hooks.slack-gov.com",
        )
    };
    let url = Url::parse(value).map_err(|_| invalid())?;
    let segments: Vec<_> = url.path().split('/').collect();
    if value.len() > 2048
        || url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some("hooks.slack.com" | "hooks.slack-gov.com")
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port().is_some()
        || segments.len() != 5
        || segments[1] != "services"
        || segments[2..].iter().any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
    {
        return Err(invalid());
    }
    Ok(url)
}

async fn read(
    State(app): State<Arc<SignalAlerts>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<SignalAlertView>, Error> {
    app.authorize(&headers, &method).await?;
    Ok(Json(app.view().await?))
}

async fn save(
    State(app): State<Arc<SignalAlerts>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<SignalAlertView>, Error> {
    app.authorize(&headers, &method).await?;
    let request: SignalAlertRequest =
        serde_json::from_slice(&body).map_err(|_| Error::Invalid("Invalid alert settings"))?;
    let previous = app.store.read(SETTINGS).await?;
    if request.revision != previous.head.revision {
        return Err(Error::Conflict);
    }
    let before: Settings = if previous.value.is_null() {
        Settings::default()
    } else {
        serde_json::from_value(previous.value.clone())?
    };
    let signals = Signals(app.store.clone())
        .get_config()
        .await
        .map_err(|_| Error::Invalid("Save your signals before setting up alerts"))?;
    let selected: BTreeSet<_> = request.signal_ids.iter().collect();
    if selected.len() != request.signal_ids.len()
        || request
            .signal_ids
            .iter()
            .any(|id| !signals.signals.iter().any(|signal| &signal.id == id))
    {
        return Err(Error::Invalid("Choose from your saved signals"));
    }
    if request.enabled && (signals.model.is_empty() || request.signal_ids.is_empty()) {
        return Err(Error::Invalid(
            "Save a signal model and select at least one signal",
        ));
    }
    let replace = request
        .webhook_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty());
    let encrypted = if replace || request.enabled {
        let url = app.webhook(request.webhook_url.as_deref(), &before)?;
        if replace {
            app.secret.encrypt(url.as_str())?
        } else {
            before.webhook.clone()
        }
    } else {
        before.webhook.clone()
    };
    let restart = request.enabled && (!before.enabled || replace);
    let settings = Settings {
        enabled: request.enabled,
        signal_ids: request.signal_ids,
        webhook: encrypted,
        generation: if restart {
            uuid::Uuid::new_v4().to_string()
        } else {
            before.generation
        },
        activated_at: if restart {
            Some(Utc::now())
        } else {
            before.activated_at
        },
    };
    app.store
        .commit(vec![Change {
            previous,
            value: serde_json::to_value(settings)?,
        }])
        .await?;
    Ok(Json(app.view().await?))
}

async fn test(
    State(app): State<Arc<SignalAlerts>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<Value>, Error> {
    app.authorize(&headers, &method).await?;
    let request: SignalAlertTest =
        serde_json::from_slice(&body).map_err(|_| Error::Invalid("Invalid test request"))?;
    let settings = storage::settings(&app.store).await?;
    let url = app.webhook(request.webhook_url.as_deref(), &settings)?;
    let result = send(&app.client, &url, json!({"text":"Lens signal alert test. Slack is connected. Save your alert settings in Lens to receive matches.","mrkdwn":false,"unfurl_links":false,"unfurl_media":false})).await;
    match result {
        Outcome::Delivered => Ok(Json(json!({"delivered":true}))),
        _ => Err(Error::Delivery),
    }
}

use delivery::{Outcome, send};

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::slack("https://hooks.slack.com/services/EXAMPLE/EXAMPLE/FAKE", true)]
    #[case::government("https://hooks.slack-gov.com/services/EXAMPLE/EXAMPLE/FAKE", true)]
    #[case::insecure("http://hooks.slack.com/services/EXAMPLE/EXAMPLE/FAKE", false)]
    #[case::other_host("https://example.com/services/EXAMPLE/EXAMPLE/FAKE", false)]
    #[case::suffix(
        "https://hooks.slack.com.example.com/services/EXAMPLE/EXAMPLE/FAKE",
        false
    )]
    #[case::userinfo("https://user@hooks.slack.com/services/EXAMPLE/EXAMPLE/FAKE", false)]
    #[case::query("https://hooks.slack.com/services/EXAMPLE/EXAMPLE/FAKE?x=y", false)]
    #[case::path("https://hooks.slack.com/services/", false)]
    fn validates_the_destination_without_echoing_credentials(
        #[case] value: &str,
        #[case] valid: bool,
    ) {
        let result = webhook(value);
        assert_eq!(result.is_ok(), valid);
        if let Err(error) = result {
            assert!(!error.to_string().contains("FAKE"));
        }
    }
}
