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
            .find(&SessionId::for_token("parity-expired-session"))
            .await
            .unwrap()
            .unwrap()
            .expires_at
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
        json!({"user_id":"lens-admin", "user_name":"Setup token", "user_role":"proxy_admin"})
    );
    let expires = Sessions(database.store.clone())
        .find(&SessionId::for_token(token))
        .await
        .unwrap()
        .unwrap()
        .expires_at;
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
        json!({"user_id":"lens-admin", "user_name":"Setup token", "user_role":"proxy_admin"})
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

fn litellm_login_token(audience: &str) -> String {
    let now = chrono::Utc::now().timestamp();
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &json!({
            "iss": "litellm", "aud": audience, "sub": "user", "iat": now, "exp": now + 60,
            "identity": {"user_id": "user", "user_name": "user@example.com", "user_role": "internal_user", "team_id": null, "org_id": null,
                "token": "hash", "models": [], "log_team_ids": ["alpha"]}
        }),
        &jsonwebtoken::EncodingKey::from_secret(SECRET.as_bytes()),
    )
    .unwrap()
}

#[rstest]
#[tokio::test]
async fn litellm_login_token_opens_a_browser_session_as_the_gateway_user(
    #[future(awt)] database: Database,
) {
    let server = database.serve(false).await;
    let login = server
        .client
        .post(server.endpoint())
        .json(&json!({"token": litellm_login_token("litellm-lens-login")}))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 200);
    let cookie = login.headers()["set-cookie"].to_str().unwrap().to_owned();
    assert!(cookie.contains("HttpOnly; Max-Age=3600; Path=/; SameSite=strict"));
    let pair = cookie.split(';').next().unwrap().to_owned();
    let info = server
        .client
        .get(server.endpoint())
        .header("cookie", &pair)
        .send()
        .await
        .unwrap();
    assert_eq!(
        info.json::<Value>().await.unwrap(),
        json!({"user_id":"user", "user_name":"user@example.com", "user_role":"internal_user"})
    );
    let request_token = server
        .client
        .post(server.endpoint())
        .json(&json!({"token": litellm_login_token("litellm-lens")}))
        .send()
        .await
        .unwrap();
    assert_eq!(request_token.status(), 401);
}

#[rstest]
#[case::configured(Some("https://gateway.test"), json!({"litellm_login_url": "https://gateway.test/lens/login"}))]
#[case::not_configured(None, json!({"litellm_login_url": null}))]
#[tokio::test]
async fn sign_in_methods_are_public_and_name_the_gateway_login(
    #[future(awt)] database: Database,
    #[case] gateway: Option<&'static str>,
    #[case] methods: Value,
) {
    let server = database.serve_with_gateway(gateway).await;
    let response = server
        .client
        .get(server.url.join("/auth/methods").unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>().await.unwrap(), methods);
}

async fn login_url(server: &support::Server) -> Value {
    server
        .client
        .get(server.url.join("/auth/methods").unwrap())
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["litellm_login_url"]
        .clone()
}

async fn connect(server: &support::Server, token: &str, gateway: &str) -> reqwest::Response {
    server
        .client
        .post(server.url.join("/auth/gateway").unwrap())
        .json(&json!({"token": token, "gateway_url": gateway}))
        .send()
        .await
        .unwrap()
}

#[rstest]
#[tokio::test]
async fn connecting_a_gateway_turns_on_litellm_login_and_survives_a_restart(
    #[future(awt)] database: Database,
) {
    let server = database.serve_standalone().await;
    assert_eq!(login_url(&server).await, Value::Null);
    let response = connect(&server, ADMIN, "https://gateway.test/v1").await;
    assert_eq!(response.status(), 200);
    let connected = response.json::<Value>().await.unwrap();
    assert_eq!(
        connected["litellm_login_url"],
        "https://gateway.test/lens/login"
    );
    let lens = server.url.as_str().trim_end_matches('/');
    let lines = connected["gateway_environment"].as_array().unwrap();
    assert_eq!(lines[0], format!("LITELLM_LENS_URL={lens}"));
    assert_eq!(lines[1], format!("LITELLM_LENS_PUBLIC_URL={lens}"));
    let secret = lines[2]
        .as_str()
        .unwrap()
        .strip_prefix("LENS_GATEWAY_SECRET=")
        .unwrap();
    assert!(secret.len() >= 32);
    assert_eq!(
        login_url(&database.serve_standalone().await).await,
        "https://gateway.test/lens/login"
    );
}

#[rstest]
#[case::wrong_token("another-admin-token-32-characters-long", "https://gateway.test", 401)]
#[case::not_a_url(ADMIN, "gateway", 422)]
#[tokio::test]
async fn connecting_a_gateway_needs_the_admin_token_and_a_url(
    #[future(awt)] database: Database,
    #[case] token: &str,
    #[case] gateway: &str,
    #[case] status: u16,
) {
    let server = database.serve_standalone().await;
    assert_eq!(connect(&server, token, gateway).await.status(), status);
    assert_eq!(login_url(&server).await, Value::Null);
}

#[rstest]
#[tokio::test]
async fn a_gateway_set_in_the_environment_is_not_replaced(#[future(awt)] database: Database) {
    let server = database
        .serve_with_gateway(Some("https://gateway.test"))
        .await;
    assert_eq!(
        connect(&server, ADMIN, "https://other.test").await.status(),
        409
    );
    assert_eq!(login_url(&server).await, "https://gateway.test/lens/login");
}
