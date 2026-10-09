#[allow(
    dead_code,
    reason = "state fixtures are shared with the state integration suite"
)]
#[path = "state/support.rs"]
mod support;

use chrono::{TimeDelta, Utc};
use lens_contract::github::{
    Authorization, AuthorizationState, BrokerConnection, BrokerHandshake, BrokerHandshakeState,
    Connection, Owner,
};
use litellm_storage_clickhouse::{Error, github::GitHubStore};
use rstest::{fixture, rstest};
use serde_json::json;
use support::{Database, database};

#[fixture]
fn authorization() -> Authorization {
    Authorization {
        id: uuid::Uuid::new_v4().to_string(),
        owner: Owner {
            scope: "team".into(),
            subject: "user".into(),
        },
        agent: "assistant".into(),
        expires_at: Utc::now() + TimeDelta::minutes(10),
        state: AuthorizationState::Pending,
    }
}

#[fixture]
fn connection() -> Connection {
    Connection {
        agent: "assistant".into(),
        repository_id: 42,
        repository: "owner/project".into(),
        installation_id: 7,
        default_branch: "main".into(),
        connected_at: Utc::now(),
    }
}

fn handoff(authorization: &Authorization) -> BrokerHandshake {
    BrokerHandshake {
        id: authorization.id.clone(),
        agent: authorization.agent.clone(),
        lens_origin: "https://lens.example".into(),
        redirect_uri: "https://lens.example/lens/github/service/callback".into(),
        local_state: "local-browser-state".into(),
        code_challenge: "pkce-challenge".into(),
        expires_at: authorization.expires_at,
        browser_claimed: false,
        browser_hash: String::new(),
        state: BrokerHandshakeState::Pending,
    }
}

async fn ready(store: &GitHubStore, authorization: &Authorization) {
    store
        .create_handshake(authorization, &handoff(authorization))
        .await
        .unwrap();
    let stored = store.handshake(&authorization.id).await.unwrap().unwrap();
    store.claim_handshake(stored, "a".repeat(64)).await.unwrap();
    let stored = store
        .authorization(&authorization.id)
        .await
        .unwrap()
        .unwrap();
    store
        .transition(
            stored,
            AuthorizationState::Ready {
                repositories: vec![],
            },
        )
        .await
        .unwrap();
}

async fn selected(store: &GitHubStore, authorization: &Authorization, connection: &Connection) {
    ready(store, authorization).await;
    store
        .select_repository(
            store
                .authorization(&authorization.id)
                .await
                .unwrap()
                .unwrap(),
            store.handshake(&authorization.id).await.unwrap().unwrap(),
            "code-hash".into(),
            connection,
        )
        .await
        .unwrap();
}

fn grant(connection: &Connection) -> BrokerConnection {
    BrokerConnection {
        id: uuid::Uuid::new_v4().to_string(),
        lens_origin: "https://lens.example".into(),
        connection: connection.clone(),
        capability_hash: "b".repeat(64),
        revoked: false,
    }
}

#[rstest]
#[case::oauth(AuthorizationState::Pending)]
#[case::install(AuthorizationState::Installing)]
#[tokio::test]
async fn creates_matched_handshake_and_authorization_once(
    #[future(awt)] database: Database,
    authorization: Authorization,
    #[case] state: AuthorizationState,
) {
    let store = GitHubStore(database.store.clone());
    let authorization = Authorization {
        state,
        ..authorization
    };
    let handshake = handoff(&authorization);
    store
        .create_handshake(&authorization, &handshake)
        .await
        .unwrap();
    assert_eq!(
        store
            .authorization(&authorization.id)
            .await
            .unwrap()
            .unwrap()
            .authorization,
        authorization
    );
    assert_eq!(
        store
            .handshake(&authorization.id)
            .await
            .unwrap()
            .unwrap()
            .handshake,
        handshake
    );
    assert!(matches!(
        store.create_handshake(&authorization, &handshake).await,
        Err(Error::StateExists)
    ));
}

#[rstest]
#[tokio::test]
async fn browser_claim_has_one_winner_and_cannot_be_replaced(
    #[future(awt)] database: Database,
    authorization: Authorization,
) {
    let store = GitHubStore(database.store.clone());
    let independent = GitHubStore(database.independent());
    store
        .create_handshake(&authorization, &handoff(&authorization))
        .await
        .unwrap();
    let first = store.handshake(&authorization.id).await.unwrap().unwrap();
    let second = independent
        .handshake(&authorization.id)
        .await
        .unwrap()
        .unwrap();
    let (first_result, second_result) = tokio::join!(
        store.claim_handshake(first, "a".repeat(64)),
        independent.claim_handshake(second, "b".repeat(64)),
    );
    let expected = match (first_result, second_result) {
        (Ok(()), Err(Error::StateConflict)) => "a".repeat(64),
        (Err(Error::StateConflict), Ok(())) => "b".repeat(64),
        other => panic!("one browser must win: {other:?}"),
    };
    let claimed = store.handshake(&authorization.id).await.unwrap().unwrap();
    assert!(claimed.handshake.browser_claimed);
    assert_eq!(claimed.handshake.browser_hash, expected);
    assert!(matches!(
        store.claim_handshake(claimed, "c".repeat(64)).await,
        Err(Error::StateConflict)
    ));
    assert_eq!(
        store
            .handshake(&authorization.id)
            .await
            .unwrap()
            .unwrap()
            .handshake
            .browser_hash,
        expected
    );
}

#[rstest]
#[tokio::test]
async fn unclaimed_browser_cannot_select_repository(
    #[future(awt)] database: Database,
    authorization: Authorization,
    connection: Connection,
) {
    let store = GitHubStore(database.store.clone());
    let authorization = Authorization {
        state: AuthorizationState::Ready {
            repositories: vec![],
        },
        ..authorization
    };
    store
        .create_handshake(&authorization, &handoff(&authorization))
        .await
        .unwrap();
    let result = store
        .select_repository(
            store
                .authorization(&authorization.id)
                .await
                .unwrap()
                .unwrap(),
            store.handshake(&authorization.id).await.unwrap().unwrap(),
            "code-hash".into(),
            &connection,
        )
        .await;
    assert!(matches!(result, Err(Error::StateConflict)));
    assert_eq!(
        store
            .handshake(&authorization.id)
            .await
            .unwrap()
            .unwrap()
            .handshake
            .state,
        BrokerHandshakeState::Pending
    );
    assert_eq!(
        store
            .authorization(&authorization.id)
            .await
            .unwrap()
            .unwrap()
            .authorization
            .state,
        authorization.state
    );
}

#[rstest]
#[tokio::test]
async fn selection_rejects_another_authorization_and_stale_replay(
    #[future(awt)] database: Database,
    authorization: Authorization,
    connection: Connection,
) {
    let store = GitHubStore(database.store.clone());
    ready(&store, &authorization).await;
    let other = Authorization {
        id: uuid::Uuid::new_v4().to_string(),
        ..authorization.clone()
    };
    ready(&store, &other).await;
    assert!(matches!(
        store
            .select_repository(
                store.authorization(&other.id).await.unwrap().unwrap(),
                store.handshake(&authorization.id).await.unwrap().unwrap(),
                "wrong-code".into(),
                &connection,
            )
            .await,
        Err(Error::StateConflict)
    ));
    let stale_authorization = store
        .authorization(&authorization.id)
        .await
        .unwrap()
        .unwrap();
    let stale_handshake = store.handshake(&authorization.id).await.unwrap().unwrap();
    store
        .select_repository(
            store
                .authorization(&authorization.id)
                .await
                .unwrap()
                .unwrap(),
            store.handshake(&authorization.id).await.unwrap().unwrap(),
            "right-code".into(),
            &connection,
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .select_repository(
                stale_authorization,
                stale_handshake,
                "replayed-code".into(),
                &connection
            )
            .await,
        Err(Error::StateConflict)
    ));
    assert_eq!(
        store
            .handshake(&authorization.id)
            .await
            .unwrap()
            .unwrap()
            .handshake
            .state,
        BrokerHandshakeState::Selected {
            code_hash: "right-code".into(),
            connection
        }
    );
    assert_eq!(
        store
            .authorization(&authorization.id)
            .await
            .unwrap()
            .unwrap()
            .authorization
            .state,
        AuthorizationState::Connected
    );
    assert!(matches!(
        store
            .authorization(&other.id)
            .await
            .unwrap()
            .unwrap()
            .authorization
            .state,
        AuthorizationState::Ready { .. }
    ));
}

#[rstest]
#[case::origin("https://other.example", 42)]
#[case::repository("https://lens.example", 99)]
#[tokio::test]
async fn redemption_rejects_changed_binding_without_consuming_code(
    #[future(awt)] database: Database,
    authorization: Authorization,
    connection: Connection,
    #[case] origin: &str,
    #[case] repository_id: u64,
) {
    let store = GitHubStore(database.store.clone());
    selected(&store, &authorization, &connection).await;
    let bound = grant(&Connection {
        repository_id,
        ..connection
    });
    let bound = BrokerConnection {
        lens_origin: origin.into(),
        ..bound
    };
    assert!(matches!(
        store
            .redeem_handshake(
                store.handshake(&authorization.id).await.unwrap().unwrap(),
                &bound
            )
            .await,
        Err(Error::StateConflict)
    ));
    assert!(store.broker_connection(&bound.id).await.unwrap().is_none());
    assert!(matches!(
        store
            .handshake(&authorization.id)
            .await
            .unwrap()
            .unwrap()
            .handshake
            .state,
        BrokerHandshakeState::Selected { .. }
    ));
}

#[rstest]
#[tokio::test]
async fn redemption_has_one_winner_and_revocation_preserves_binding(
    #[future(awt)] database: Database,
    authorization: Authorization,
    connection: Connection,
) {
    let store = GitHubStore(database.store.clone());
    let independent = GitHubStore(database.independent());
    selected(&store, &authorization, &connection).await;
    let first = store.handshake(&authorization.id).await.unwrap().unwrap();
    let second = independent
        .handshake(&authorization.id)
        .await
        .unwrap()
        .unwrap();
    let first_grant = grant(&connection);
    let second_grant = grant(&connection);
    let results = tokio::join!(
        store.redeem_handshake(first, &first_grant),
        independent.redeem_handshake(second, &second_grant)
    );
    let (winner, loser) = match results {
        (Ok(()), Err(Error::StateConflict)) => (first_grant, second_grant),
        (Err(Error::StateConflict), Ok(())) => (second_grant, first_grant),
        other => panic!("one redemption must win: {other:?}"),
    };
    assert!(store.broker_connection(&loser.id).await.unwrap().is_none());
    assert_eq!(
        store
            .handshake(&authorization.id)
            .await
            .unwrap()
            .unwrap()
            .handshake
            .state,
        BrokerHandshakeState::Redeemed
    );
    let stored = store.broker_connection(&winner.id).await.unwrap().unwrap();
    let stale = store.broker_connection(&winner.id).await.unwrap().unwrap();
    assert_eq!(stored.connection, winner);
    store.revoke_broker_connection(stored).await.unwrap();
    assert!(matches!(
        store.revoke_broker_connection(stale).await,
        Err(Error::StateConflict)
    ));
    assert_eq!(
        store
            .broker_connection(&winner.id)
            .await
            .unwrap()
            .unwrap()
            .connection,
        BrokerConnection {
            revoked: true,
            ..winner
        }
    );
}

#[rstest]
#[tokio::test]
async fn remote_replacement_and_disconnect_retain_only_unrevoked_ciphertexts(
    #[future(awt)] database: Database,
    authorization: Authorization,
    connection: Connection,
) {
    let store = GitHubStore(database.store.clone());
    let scope = &authorization.owner.scope;
    let agent = &authorization.agent;
    let old = json!({"version": 1, "nonce": "old-nonce", "ciphertext": "opaque-old"});
    let current = json!({"version": 1, "nonce": "new-nonce", "ciphertext": "opaque-new"});
    store.create_authorization(&authorization).await.unwrap();
    assert_eq!(
        store
            .connect_remote(
                store
                    .authorization(&authorization.id)
                    .await
                    .unwrap()
                    .unwrap(),
                &connection,
                old.clone()
            )
            .await
            .unwrap(),
        None
    );
    let replacement = Authorization {
        id: uuid::Uuid::new_v4().to_string(),
        ..authorization.clone()
    };
    store.create_authorization(&replacement).await.unwrap();
    let stale = store.authorization(&replacement.id).await.unwrap().unwrap();
    let changed = Connection {
        repository_id: 99,
        repository: "owner/replacement".into(),
        ..connection.clone()
    };
    assert_eq!(
        store
            .connect_remote(
                store.authorization(&replacement.id).await.unwrap().unwrap(),
                &changed,
                current.clone()
            )
            .await
            .unwrap(),
        Some(old.clone())
    );
    assert!(matches!(
        store
            .connect_remote(stale, &connection, json!({"ciphertext": "stale"}))
            .await,
        Err(Error::StateConflict)
    ));
    assert_eq!(store.connection(scope, agent).await.unwrap(), Some(changed));
    assert_eq!(
        store.remote_credentials(scope, agent).await.unwrap(),
        Some(current.clone())
    );
    assert_eq!(
        store
            .retired_remote_credentials(scope, agent)
            .await
            .unwrap(),
        vec![old.clone()]
    );
    assert!(
        store
            .remote_credentials("other-team", agent)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .retired_remote_credentials("other-team", agent)
            .await
            .unwrap()
            .is_empty()
    );
    store.disconnect_remote(scope, agent).await.unwrap();
    assert!(store.connection(scope, agent).await.unwrap().is_none());
    assert!(
        store
            .remote_credentials(scope, agent)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .retired_remote_credentials(scope, agent)
            .await
            .unwrap(),
        vec![old.clone(), current.clone()]
    );
    store
        .remove_retired_remote_credentials(scope, agent, &old)
        .await
        .unwrap();
    store
        .remove_retired_remote_credentials(scope, agent, &json!({"ciphertext": "unrelated"}))
        .await
        .unwrap();
    assert_eq!(
        store
            .retired_remote_credentials(scope, agent)
            .await
            .unwrap(),
        vec![current.clone()]
    );
    store.disconnect_remote(scope, agent).await.unwrap();
    assert_eq!(
        store
            .retired_remote_credentials(scope, agent)
            .await
            .unwrap(),
        vec![current.clone()]
    );
    store
        .remove_retired_remote_credentials(scope, agent, &current)
        .await
        .unwrap();
    assert!(
        store
            .retired_remote_credentials(scope, agent)
            .await
            .unwrap()
            .is_empty()
    );
}
