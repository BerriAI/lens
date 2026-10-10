#[path = "sessions/support.rs"]
mod support;

use lens_auth::{SessionId, SessionRepository};
use litellm_storage_clickhouse::sessions::Sessions;
use rstest::rstest;
use serde_json::{Value, json};
use std::path::PathBuf;
use support::{ADMIN, Database, SECRET, database};

#[rstest]
#[case::auth("auth", 18)]
#[case::boundaries("auth-boundaries", 27)]
#[tokio::test]
async fn python_fixtures_replay_over_real_http_and_clickhouse(
    #[future(awt)] database: Database,
    #[case] group: &str,
    #[case] count: usize,
) {
    database.seed_expired().await;
    let server = database.serve(false).await;
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../parity/fixtures")
        .join(group);
    let report = lens_parity::replay_fixtures(
        &server.url,
        &fixtures,
        &lens_parity::Tokens::new(ADMIN, SECRET),
    )
    .await
    .unwrap();
    assert!(report.failures.is_empty(), "{:#?}", report.failures);
    assert_eq!(report.passed, count);
    assert!(
        Sessions(database.store.clone())
            .expires_at(&SessionId::for_token("parity-expired-session"))
            .await
            .unwrap()
            .unwrap()
            < chrono::Utc::now()
    );
}

#[rstest]
#[case::http(false)]
#[case::https(true)]
#[tokio::test]
async fn browser_session_survives_server_restart_and_cannot_be_replayed_after_logout(
    #[future(awt)] database: Database,
    #[case] secure: bool,
) {
    let server = database.serve(secure).await;
    let before = chrono::Utc::now();
    let login = server
        .client
        .post(server.endpoint())
        .json(&json!({"token": ADMIN}))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 200);
    let cookie = login.headers()["set-cookie"].to_str().unwrap().to_owned();
    assert!(cookie.contains("HttpOnly; Max-Age=28800; Path=/; SameSite=strict"));
    assert_eq!(cookie.ends_with("; Secure"), secure);
    let pair = cookie.split(';').next().unwrap().to_owned();
    let token = pair.strip_prefix("lens_session=").unwrap();
    assert_eq!(token.len(), 64);
    assert_eq!(
        login.json::<Value>().await.unwrap(),
        json!({"user_id":"lens-admin", "user_role":"proxy_admin"})
    );
    let expires = Sessions(database.store.clone())
        .expires_at(&SessionId::for_token(token))
        .await
        .unwrap()
        .unwrap();
    assert!(expires >= before + chrono::Duration::hours(8));
    assert!(expires <= chrono::Utc::now() + chrono::Duration::hours(8));
    drop(server);
    let restarted = database.serve(secure).await;
    let info = restarted
        .client
        .get(restarted.endpoint())
        .header("cookie", &pair)
        .send()
        .await
        .unwrap();
    assert_eq!(info.status(), 200);
    assert_eq!(
        info.json::<Value>().await.unwrap(),
        json!({"user_id":"lens-admin", "user_role":"proxy_admin"})
    );
    let denied = restarted
        .client
        .delete(restarted.endpoint())
        .header("cookie", &pair)
        .header("origin", "https://other.test")
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403);
    let invalid_bearer = restarted
        .client
        .get(restarted.endpoint())
        .header("cookie", &pair)
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_bearer.status(), 401);
    let origin = if secure {
        "https://lens.test".to_owned()
    } else {
        restarted.url.origin().ascii_serialization()
    };
    let logout = restarted
        .client
        .delete(restarted.endpoint())
        .header("cookie", &pair)
        .header("origin", origin)
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), 204);
    assert!(logout.headers().get("content-type").is_none());
    assert!(
        logout.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .ends_with("; Max-Age=0; Path=/; SameSite=lax")
    );
    assert!(logout.bytes().await.unwrap().is_empty());
    drop(restarted);
    let restarted = database.serve(secure).await;
    let replay = restarted
        .client
        .get(restarted.endpoint())
        .header("cookie", pair)
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), 401);
    assert_eq!(
        replay.json::<Value>().await.unwrap(),
        json!({"detail":"Lens session has expired"})
    );
    let keys = database.store.keys("session/", "", 10).await.unwrap();
    assert_eq!(keys.len(), 1);
    let record = database.store.read(&keys[0]).await.unwrap();
    assert!(record.value.is_null());
    assert_eq!(record.head.revision, 2);
}

#[rstest]
#[tokio::test]
async fn restricted_identity_survives_server_restart(#[future(awt)] database: Database) {
    let identity = lens_contract::auth::Identity {
        user_id: Some("google:subject-1".into()),
        user_role: lens_contract::auth::Role::InternalUserViewer,
        ..Default::default()
    };
    Sessions(database.store.clone())
        .create_identity(
            &SessionId::for_token("google-session"),
            chrono::Utc::now() + chrono::Duration::hours(1),
            &identity,
        )
        .await
        .unwrap();
    let server = database.serve(true).await;
    let response = server
        .client
        .get(server.endpoint())
        .header("cookie", "lens_session=google-session")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"user_id":"google:subject-1", "user_role":"internal_user_viewer"})
    );
    drop(server);
    let restarted = database.serve(true).await;
    let response = restarted
        .client
        .get(restarted.endpoint())
        .header("cookie", "lens_session=google-session")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"user_id":"google:subject-1", "user_role":"internal_user_viewer"})
    );
}
