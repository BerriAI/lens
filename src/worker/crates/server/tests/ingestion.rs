mod identity;
#[path = "sessions/support.rs"]
pub mod support;

use std::sync::{Arc, Mutex};

use chrono::Utc;
use lens_auth::{IngestionError, ingestion::Ingestion};
use lens_contract::{
    auth::{Identity, Role},
    ingestion::{IngestionKey, IngestionKeyCreated, IngestionSnapshot},
};
use lens_server::ingestion::CredentialPublisher;
use litellm_storage_clickhouse::ingestion::IngestionKeys;
use rstest::rstest;
use serde_json::{Value, json};
use support::{ADMIN, Database, Server, database};

struct Publisher {
    keys: Ingestion<IngestionKeys>,
    snapshots: Mutex<Vec<IngestionSnapshot>>,
    active: bool,
}

impl CredentialPublisher for Publisher {
    async fn refresh(&self) -> Result<bool, IngestionError> {
        let snapshot = self.keys.current_snapshot(Utc::now()).await?;
        self.snapshots.lock().unwrap().push(snapshot);
        Ok(self.active)
    }
}

async fn serve(database: &Database, active: bool) -> (Server, Arc<Publisher>) {
    let keys = Ingestion(IngestionKeys(database.store.clone()));
    let publisher = Arc::new(Publisher {
        keys: keys.clone(),
        snapshots: Mutex::new(Vec::new()),
        active,
    });
    let server = database
        .serve_router(false, |auth| {
            lens_server::sessions::router_with_auth(auth.clone()).merge(
                lens_server::ingestion::router(auth, keys, publisher.clone()),
            )
        })
        .await;
    (server, publisher)
}

#[rstest]
#[case::installed(true)]
#[case::pending(false)]
#[tokio::test]
async fn created_keys_publish_once_hide_plaintext_and_survive_restart_until_revoked(
    #[future(awt)] database: Database,
    #[case] active: bool,
) {
    let (server, publisher) = serve(&database, active).await;
    let created = server
        .client
        .post(server.url.join("/lens/tracing/keys").unwrap())
        .bearer_auth(ADMIN)
        .json(&json!({"team_id":"team-a"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200);
    let created: IngestionKeyCreated = created.json().await.unwrap();
    assert_eq!(created.active, active);
    assert_eq!(created.record.name, "Agent tracing");
    assert_eq!(created.record.tenant.team_id, "team-a");
    assert!(created.key.starts_with("lens-trace-"));
    assert_eq!(
        publisher.snapshots.lock().unwrap()[0].keys[0].token_hash,
        created.record.tenant.api_key_hash
    );
    drop(server);
    let (server, publisher) = serve(&database, true).await;
    let listed = server
        .client
        .get(server.url.join("/lens/tracing/keys").unwrap())
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), 200);
    let text = listed.text().await.unwrap();
    assert!(!text.contains(&created.key));
    assert_eq!(
        serde_json::from_str::<Vec<IngestionKey>>(&text).unwrap(),
        vec![created.record.clone()]
    );
    let revoked = server
        .client
        .delete(
            server
                .url
                .join(&format!("/lens/tracing/keys/{}", created.record.id))
                .unwrap(),
        )
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), 200);
    assert_eq!(revoked.json::<Value>().await.unwrap(), true);
    assert!(publisher.snapshots.lock().unwrap()[0].keys.is_empty());
    drop(server);
    let (server, _) = serve(&database, true).await;
    let listed = server
        .client
        .get(server.url.join("/lens/tracing/keys").unwrap())
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(listed.json::<Value>().await.unwrap(), json!([]));
}

#[rstest]
#[case::viewer_read(Role::ProxyAdminViewer, "GET", 200)]
#[case::viewer_create(Role::ProxyAdminViewer, "POST", 403)]
#[case::viewer_revoke(Role::ProxyAdminViewer, "DELETE", 403)]
#[case::user_read(Role::InternalUser, "GET", 403)]
#[case::user_create(Role::InternalUser, "POST", 403)]
#[tokio::test]
async fn key_permissions_follow_authenticated_role(
    #[future(awt)] database: Database,
    #[case] role: Role,
    #[case] method: &str,
    #[case] status: u16,
) {
    let (server, publisher) = serve(&database, true).await;
    let token = identity::delegated(Identity {
        user_id: Some("user".into()),
        user_role: role,
        ..Identity::default()
    });
    let path = if method == "DELETE" {
        "/lens/tracing/keys/missing"
    } else {
        "/lens/tracing/keys"
    };
    let response = server
        .client
        .request(method.parse().unwrap(), server.url.join(path).unwrap())
        .bearer_auth(token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    assert!(publisher.snapshots.lock().unwrap().is_empty());
}

#[rstest]
#[case::unknown_field(json!({"extra":1}), "extra_forbidden", json!(["body","extra"]))]
#[case::empty_name(json!({"name":""}), "string_too_short", json!(["body","name"]))]
#[case::missing_timezone(json!({"expires_at":"2030-01-01T00:00:00"}), "timezone_aware", json!(["body","expires_at"]))]
#[case::invalid_datetime(json!({"expires_at":"bad"}), "datetime_from_date_parsing", json!(["body","expires_at"]))]
#[tokio::test]
async fn invalid_key_input_does_not_persist_or_publish(
    #[future(awt)] database: Database,
    #[case] body: Value,
    #[case] kind: &str,
    #[case] loc: Value,
) {
    let (server, publisher) = serve(&database, true).await;
    let response = server
        .client
        .post(server.url.join("/lens/tracing/keys").unwrap())
        .bearer_auth(ADMIN)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["detail"][0]["type"], kind);
    assert_eq!(body["detail"][0]["loc"], loc);
    assert!(publisher.snapshots.lock().unwrap().is_empty());
    assert!(
        publisher
            .keys
            .current_snapshot(Utc::now())
            .await
            .unwrap()
            .keys
            .is_empty()
    );
}

#[rstest]
#[tokio::test]
async fn expired_key_request_is_rejected_before_storage(#[future(awt)] database: Database) {
    let (server, publisher) = serve(&database, true).await;
    let response = server
        .client
        .post(server.url.join("/lens/tracing/keys").unwrap())
        .bearer_auth(ADMIN)
        .json(&json!({"expires_at":0}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":"Choose an expiry in the future"})
    );
    assert!(publisher.snapshots.lock().unwrap().is_empty());
}

#[rstest]
#[case::missing_origin(None, 403)]
#[case::foreign_origin(Some("https://foreign.test"), 403)]
#[case::lens_origin(Some("self"), 200)]
#[tokio::test]
async fn cookie_key_writes_require_the_lens_origin(
    #[future(awt)] database: Database,
    #[case] origin: Option<&str>,
    #[case] status: u16,
) {
    let (server, publisher) = serve(&database, true).await;
    let login = server
        .client
        .post(server.endpoint())
        .header("origin", server.url.as_str().trim_end_matches('/'))
        .json(&json!({"token":ADMIN}))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 200);
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let request = server
        .client
        .post(server.url.join("/lens/tracing/keys").unwrap())
        .header("cookie", cookie)
        .json(&json!({}));
    let request = if let Some(origin) = origin {
        request.header(
            "origin",
            if origin == "self" {
                server.url.as_str().trim_end_matches('/')
            } else {
                origin
            },
        )
    } else {
        request
    };
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), status);
    assert_eq!(
        publisher.snapshots.lock().unwrap().len(),
        usize::from(status == 200)
    );
}
