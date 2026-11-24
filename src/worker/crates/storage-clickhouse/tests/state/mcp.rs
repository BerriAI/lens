use super::support;

use chrono::Utc;
use lens_auth::mcp::{self, ActivityQuery, McpRepository};
use lens_contract::mcp::{McpCall, McpCreateRequest, McpOutcome};
use litellm_storage_clickhouse::sessions::Sessions;
use rstest::rstest;
use support::{Database, database, isolated_database};

fn query(owner: Option<&str>, before: Option<(i64, String)>) -> ActivityQuery {
    ActivityQuery {
        owner_id: owner.map(str::to_owned),
        connection_id: None,
        from_ms: Utc::now().timestamp_millis() - 3600000,
        to_ms: Utc::now().timestamp_millis() + 1000,
        before,
    }
}

fn call(index: usize, owner: &str) -> McpCall {
    McpCall {
        id: uuid::Uuid::new_v4().to_string(),
        at_ms: Utc::now().timestamp_millis(),
        connection_id: owner.into(),
        connection_name: "Connection".into(),
        owner_id: owner.into(),
        owner_name: owner.into(),
        client: "Codex".into(),
        method: "tools/call".into(),
        tool: Some("list_traces".into()),
        outcome: if index.is_multiple_of(2) {
            McpOutcome::Success
        } else {
            McpOutcome::ToolError
        },
        latency_ms: 1,
    }
}

async fn seed(repository: &Sessions) {
    for index in 0..105 {
        repository.mcp_record(&call(index, "alice")).await.unwrap();
    }
    repository.mcp_record(&call(0, "bob")).await.unwrap();
}

#[rstest]
#[tokio::test]
async fn usage_aggregates_the_window_and_cursor_pages_do_not_leak_other_owners(
    #[future(awt)] database: Database,
) {
    let repository = Sessions(database.store.clone());
    seed(&repository).await;
    let first = repository
        .mcp_activity(query(Some("alice"), None))
        .await
        .unwrap();
    assert_eq!(first.calls.len(), 100);
    assert_eq!(first.usage.len(), 1);
    assert_eq!(first.usage[0].calls, 105);
    assert_eq!(first.usage[0].errors, 52);
    assert_eq!(first.usage[0].owner_id, "alice");
    let last = first.calls.last().unwrap();
    assert_eq!(
        first.next_cursor,
        Some(format!("{}:{}", last.at_ms, last.id))
    );
    let second = repository
        .mcp_activity(query(Some("alice"), Some((last.at_ms, last.id.clone()))))
        .await
        .unwrap();
    assert_eq!(second.calls.len(), 5);
    assert!(second.next_cursor.is_none());
    assert_eq!(second.usage[0].calls, 105);
    let first_ids = first
        .calls
        .iter()
        .map(|call| &call.id)
        .collect::<std::collections::HashSet<_>>();
    let second_ids = second
        .calls
        .iter()
        .map(|call| &call.id)
        .collect::<std::collections::HashSet<_>>();
    assert!(first_ids.is_disjoint(&second_ids));
    let all = repository.mcp_activity(query(None, None)).await.unwrap();
    assert_eq!(all.usage.len(), 2);
}

#[rstest]
#[tokio::test]
async fn connections_and_activity_survive_clickhouse_restart(
    #[future(awt)] isolated_database: Database,
) {
    let repository = Sessions(isolated_database.store.clone());
    let created = mcp::create(
        &repository,
        &lens_auth::local_admin(),
        McpCreateRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            name: "Restart test".into(),
            client: "Codex".into(),
            owner_label: None,
            expires_in_days: 1,
        },
        Utc::now(),
    )
    .await
    .unwrap();
    repository.mcp_record(&call(0, "alice")).await.unwrap();
    let restarted = Sessions(isolated_database.restart().await);
    let records = restarted.mcp_connections().await.unwrap();
    assert_eq!(records.len(), 1);
    mcp::check_token(&records[0], created.token.as_deref().unwrap()).unwrap();
    assert_eq!(
        restarted
            .mcp_activity(query(Some("alice"), None))
            .await
            .unwrap()
            .usage[0]
            .calls,
        1
    );
}
