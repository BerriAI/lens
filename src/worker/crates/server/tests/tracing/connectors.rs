use super::*;
use chrono::{Duration, Utc};
use lens_auth::mcp::{self, McpRepository};
use lens_contract::mcp::McpCreateRequest;
use litellm_storage_clickhouse::sessions::Sessions;

fn member(owner: &str, team: &str, role: Role) -> Identity {
    Identity {
        user_id: Some(owner.into()),
        user_name: Some(owner.into()),
        token: Some(format!("test-key-hash-{owner}")),
        log_team_ids: vec![team.into()],
        user_role: role,
        ..Identity::default()
    }
}

fn request() -> Value {
    json!({"request_id": uuid::Uuid::new_v4().to_string(), "name":"My agent", "client":"Codex", "expires_in_days":90})
}

async fn create(server: &Server, token: &str) -> Value {
    let response = server
        .client
        .post(server.url.join("/lens/connectors/mcp").unwrap())
        .bearer_auth(token)
        .json(&request())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    response.json().await.unwrap()
}

async fn overview(server: &Server, token: &str) -> Value {
    let response = server
        .client
        .get(server.url.join("/lens/connectors/mcp").unwrap())
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.json().await.unwrap()
}

async fn named(server: &Server, id: &str, token: &str, message: Value) -> reqwest::Response {
    server
        .client
        .post(server.url.join(&format!("/lens/mcp/{id}")).unwrap())
        .bearer_auth(token)
        .json(&message)
        .send()
        .await
        .unwrap()
}

#[rstest]
#[tokio::test]
async fn mcp_connection_token_is_once_only_and_revocation_survives_new_connections(
    #[future(awt)] database: Database,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let created = create(&server, ADMIN).await;
    let id = created["connection"]["id"].as_str().unwrap();
    let token = created["token"].as_str().unwrap();
    let response = named(
        &server,
        id,
        token,
        tool_call("list_traces", json!({"agent":"PRIVATE-ARGUMENT"})),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["result"]["isError"],
        false
    );
    let listed = overview(&server, ADMIN).await;
    assert_eq!(listed["activity"]["usage"][0]["calls"], 1);
    assert_eq!(listed["activity"]["usage"][0]["errors"], 0);
    assert!(listed["activity"]["usage"][0]["last_success_ms"].is_number());
    assert!(!listed.to_string().contains(token));
    assert!(!listed.to_string().contains("PRIVATE-ARGUMENT"));
    assert!(!listed.to_string().contains("token_hash"));
    let stored = Sessions(database.store.clone())
        .mcp_connections()
        .await
        .unwrap();
    assert!(!serde_json::to_string(&stored).unwrap().contains(token));
    let revoked = server
        .client
        .delete(
            server
                .url
                .join(&format!("/lens/connectors/mcp/{id}"))
                .unwrap(),
        )
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), 200);
    create(&server, ADMIN).await;
    let restarted = serve(&database, reader.clone()).await;
    let response = named(&restarted, id, token, tool_call("list_agents", json!({}))).await;
    assert_eq!(response.status(), 401);
    let listed = overview(&restarted, ADMIN).await;
    assert_eq!(listed["connections"].as_array().unwrap().len(), 2);
    assert_eq!(listed["activity"]["calls"][0]["outcome"], "revoked");
    assert_eq!(reader.calls.lock().unwrap().len(), 1);
}

#[rstest]
#[case::settings("/lens/connectors/mcp")]
#[case::traces("/v1/traces")]
#[case::session("/auth/session")]
#[tokio::test]
async fn mcp_token_cannot_be_used_as_an_administrator_credential(
    #[future(awt)] database: Database,
    #[case] path: &str,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let created = create(&server, ADMIN).await;
    let response = server
        .client
        .get(server.url.join(path).unwrap())
        .bearer_auth(created["token"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    assert!(reader.calls.lock().unwrap().is_empty());
}

#[rstest]
#[case::mismatched_path(true)]
#[case::invalid_secret(false)]
#[tokio::test]
async fn mcp_connection_rejects_mismatched_tokens(
    #[future(awt)] database: Database,
    #[case] mismatch: bool,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let first = create(&server, ADMIN).await;
    let second = create(&server, ADMIN).await;
    let id = first["connection"]["id"].as_str().unwrap();
    let token = if mismatch {
        second["token"].as_str().unwrap().to_owned()
    } else {
        format!("{}x", first["token"].as_str().unwrap())
    };
    let response = named(&server, id, &token, tool_call("list_agents", json!({}))).await;
    assert_eq!(response.status(), 401);
    assert!(reader.calls.lock().unwrap().is_empty());
}

#[rstest]
#[tokio::test]
async fn mcp_gateway_connection_uses_current_scope_and_isolates_owners(
    #[future(awt)] database: Database,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let alice = identity::delegated(member("alice", "old-team", Role::InternalUser));
    let bob = identity::delegated(member("bob", "other-team", Role::InternalUser));
    let created = create(&server, &alice).await;
    assert_eq!(created["token"], Value::Null);
    let id = created["connection"]["id"].as_str().unwrap();
    let updated = identity::delegated(member("alice", "new-team", Role::InternalUser));
    assert_eq!(
        named(&server, id, &updated, tool_call("list_agents", json!({})))
            .await
            .status(),
        200
    );
    assert_eq!(
        named(&server, id, &bob, tool_call("list_agents", json!({})))
            .await
            .status(),
        403
    );
    {
        let calls = reader.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].scope.team_ids, vec!["new-team"]);
        assert_eq!(calls[0].scope.user_id, "alice");
        assert!(!calls[0].scope.all_teams);
    }
    let bobs_view = overview(&server, &bob).await;
    assert_eq!(bobs_view["connections"], json!([]));
    assert_eq!(bobs_view["activity"]["calls"], json!([]));
    assert_eq!(
        overview(&server, &alice).await["activity"]["usage"][0]["calls"],
        2
    );
    assert_eq!(
        overview(&server, ADMIN).await["connections"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let revoke = server
        .client
        .delete(
            server
                .url
                .join(&format!("/lens/connectors/mcp/{id}"))
                .unwrap(),
        )
        .bearer_auth(&bob)
        .send()
        .await
        .unwrap();
    assert_eq!(revoke.status(), 403);
}

#[rstest]
#[case::admin_viewer(Role::ProxyAdminViewer)]
#[case::member_viewer(Role::InternalUserViewer)]
#[tokio::test]
async fn mcp_viewers_can_read_but_not_create_or_revoke(
    #[future(awt)] database: Database,
    #[case] role: Role,
) {
    let server = serve(&database, Reader::default()).await;
    let owner = identity::delegated(member("owner", "team", Role::InternalUser));
    let viewer = identity::delegated(member("owner", "team", role));
    let created = create(&server, &owner).await;
    let id = created["connection"]["id"].as_str().unwrap();
    assert_eq!(overview(&server, &viewer).await["can_create"], false);
    let created = server
        .client
        .post(server.url.join("/lens/connectors/mcp").unwrap())
        .bearer_auth(&viewer)
        .json(&request())
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 403);
    let revoked = server
        .client
        .delete(
            server
                .url
                .join(&format!("/lens/connectors/mcp/{id}"))
                .unwrap(),
        )
        .bearer_auth(&viewer)
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), 403);
}

#[rstest]
#[tokio::test]
async fn mcp_expired_token_cannot_read_or_be_recreated_with_same_request_id(
    #[future(awt)] database: Database,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let payload = request();
    let created = mcp::create(
        &Sessions(database.store.clone()),
        &lens_auth::local_admin(),
        serde_json::from_value::<McpCreateRequest>(payload.clone()).unwrap(),
        Utc::now() - Duration::days(91),
    )
    .await
    .unwrap();
    let response = named(
        &server,
        &created.connection.id,
        created.token.as_deref().unwrap(),
        tool_call("list_traces", json!({})),
    )
    .await;
    assert_eq!(response.status(), 401);
    create(&server, ADMIN).await;
    let duplicate = server
        .client
        .post(server.url.join("/lens/connectors/mcp").unwrap())
        .bearer_auth(ADMIN)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate.status(), 409);
    assert!(reader.calls.lock().unwrap().is_empty());
}

#[rstest]
#[case::tool_error(tool_call("get_trace", json!({"trace_id":"private-trace"})), "tool_error")]
#[case::invalid_request(json!({"jsonrpc":"2.0","id":1,"method":"private-method"}), "invalid_request")]
#[tokio::test]
async fn mcp_audit_records_failures_without_arguments_or_unknown_names(
    #[future(awt)] database: Database,
    #[case] message: Value,
    #[case] outcome: &str,
) {
    let server = serve(&database, Reader::default()).await;
    assert_eq!(mcp(&server, Some(ADMIN), message).await.status(), 200);
    let data = overview(&server, ADMIN).await;
    assert_eq!(data["activity"]["calls"][0]["outcome"], outcome);
    assert!(!data.to_string().contains("private-"));
    assert_eq!(
        data["activity"]["calls"][0]["connection_name"],
        "Existing credential"
    );
}
