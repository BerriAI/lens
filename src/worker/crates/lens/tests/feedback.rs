mod feedback {
    pub mod support;
}

use chrono::{TimeDelta, Utc};
use feedback::support::{ADMIN, Database, SECRET, database, delegated};
use lens_contract::{
    auth::{Identity, Role},
    feedback::TraceIdentity,
    investigations::Scope,
};
use lens_server::feedback::{FeedbackStore, FeedbackWrite};
use litellm_lens::FeedbackApi;
use rstest::rstest;
use serde_json::{Value, json};
use std::path::PathBuf;

const TRACE: &str = "f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0";

#[rstest]
#[tokio::test]
async fn recorded_feedback_contracts_replay_against_real_trace_queries_and_writes(
    #[future(awt)] database: Database,
) {
    database.trace(TRACE, "alpha", "feedback-key").await;
    database
        .trace("b2acf6d7e6d141718bf596436b016f00", "alpha", "feedback-key")
        .await;
    let server = database.serve().await;
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../parity/fixtures/feedback");
    let report = lens_parity::replay_fixtures(
        &server.url,
        &fixtures,
        &lens_parity::Tokens::new(ADMIN, SECRET),
    )
    .await
    .unwrap();
    assert!(report.failures.is_empty(), "{:#?}", report.failures);
    assert_eq!(report.passed, 33);
}

#[rstest]
#[case::admin(Role::ProxyAdmin, None, None, "PUT", 200)]
#[case::viewer_write(Role::ProxyAdminViewer, None, None, "PUT", 403)]
#[case::viewer_read(Role::ProxyAdminViewer, None, None, "GET", 200)]
#[case::team_writer(Role::InternalUser, Some("alpha"), None, "PUT", 200)]
#[case::foreign_team(Role::InternalUser, Some("beta"), None, "PUT", 404)]
#[case::missing_scope(Role::InternalUser, None, None, "PUT", 403)]
#[case::internal_reader(Role::InternalUser, Some("alpha"), None, "GET", 403)]
#[case::wrong_key(Role::InternalUser, None, Some("foreign-key"), "PUT", 404)]
#[tokio::test]
async fn feedback_keeps_distinct_read_and_write_role_boundaries(
    #[future(awt)] database: Database,
    #[case] role: Role,
    #[case] team: Option<&str>,
    #[case] token: Option<&str>,
    #[case] method: &str,
    #[case] status: u16,
) {
    database.trace(TRACE, "alpha", "feedback-key").await;
    let server = database.serve().await;
    let token = delegated(Identity {
        user_role: role,
        user_id: Some("caller".into()),
        team_id: team.map(str::to_owned),
        token: token.map(str::to_owned),
        ..Identity::default()
    });
    let response = server
        .client
        .request(
            method.parse().unwrap(),
            server
                .url
                .join(&format!("/lens/feedback?trace_id={TRACE}"))
                .unwrap(),
        )
        .bearer_auth(token)
        .json(&json!({"trace_id":TRACE,"score":7}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    let body: Value = response.json().await.unwrap();
    if status == 404 {
        assert_eq!(body, json!({"detail":"Trace not found"}));
    }
    if method == "PUT" && status == 200 {
        assert_eq!(body["author"], "caller");
        assert_eq!(body["score"], 7);
    }
}

#[rstest]
#[tokio::test]
async fn updates_preserve_created_time_and_tombstones_survive_new_http_server(
    #[future(awt)] database: Database,
) {
    database.trace(TRACE, "alpha", "feedback-key").await;
    let store = FeedbackApi(database.state.clone());
    let scope = Scope {
        all_teams: true,
        ..Scope::default()
    };
    let trace = TraceIdentity {
        trace_id: TRACE.into(),
        trace_ref: String::new(),
    };
    let first = store
        .upsert(
            &scope,
            &FeedbackWrite {
                trace: trace.clone(),
                author: "reviewer".into(),
                score: 2,
                comment: "first".into(),
                at: chrono::DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).unwrap(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    let updated = store
        .upsert(
            &scope,
            &FeedbackWrite {
                trace: trace.clone(),
                author: "reviewer".into(),
                score: 9,
                comment: "updated".into(),
                at: first.updated_at + TimeDelta::milliseconds(1),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.created_at, first.created_at);
    assert!(updated.updated_at > first.updated_at);
    assert_eq!(
        store
            .for_trace(&scope, &trace)
            .await
            .unwrap()
            .unwrap()
            .feedback,
        vec![updated.clone()]
    );
    let deleted = store
        .delete(
            &scope,
            &trace,
            "reviewer",
            updated.updated_at + TimeDelta::milliseconds(1),
        )
        .await
        .unwrap();
    assert_eq!(deleted, Some(true));
    let config = database.state.storage.config.storage();
    let row=litellm_storage_clickhouse::execute_read(
        &database.state.storage.client,config.reader(),
        &format!("SELECT Comment,Score,toUnixTimestamp64Milli(CreatedAt) AS created,toUnixTimestamp64Milli(UpdatedAt) AS updated FROM `{}`.lens_feedback WHERE IsDeleted=1 ORDER BY UpdatedAt DESC LIMIT 1 SETTINGS output_format_json_quote_64bit_integers=0 FORMAT JSON",config.database()),
        &std::collections::BTreeMap::new(),
    ).await.unwrap();
    let rows: Value = serde_json::from_str(&row).unwrap();
    assert_eq!(rows["data"].as_array().unwrap().len(), 1);
    let tombstone = &rows["data"][0];
    assert_eq!(tombstone["Comment"], "");
    assert_eq!(tombstone["Score"], updated.score);
    assert_eq!(tombstone["created"], first.created_at.timestamp_millis());
    assert_eq!(
        tombstone["updated"],
        updated.updated_at.timestamp_millis() + 1
    );
    let server = database.serve().await;
    let response = server
        .client
        .get(server.url.join("/lens/feedback").unwrap())
        .query(&[("trace_id", TRACE)])
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["feedback"],
        json!([])
    );
    let restored = store
        .upsert(
            &scope,
            &FeedbackWrite {
                trace,
                author: "reviewer".into(),
                score: 10,
                comment: "new feedback".into(),
                at: updated.updated_at + TimeDelta::milliseconds(2),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(restored.created_at > first.created_at);
    assert_eq!(restored.created_at, restored.updated_at);
}

#[rstest]
#[tokio::test]
async fn ambiguous_trace_ids_require_reference_and_cannot_cross_scope(
    #[future(awt)] database: Database,
) {
    database.trace(TRACE, "alpha", "feedback-key").await;
    database.trace(TRACE, "beta", "other-key").await;
    let store = FeedbackApi(database.state.clone());
    let admin = Scope {
        all_teams: true,
        ..Scope::default()
    };
    let alpha = Scope {
        team_id: "alpha".into(),
        ..Scope::default()
    };
    let beta = Scope {
        team_id: "beta".into(),
        ..Scope::default()
    };
    let trace = TraceIdentity {
        trace_id: TRACE.into(),
        trace_ref: String::new(),
    };
    assert!(store.for_trace(&admin, &trace).await.unwrap().is_none());
    let target = store.for_trace(&alpha, &trace).await.unwrap().unwrap();
    let bound = TraceIdentity {
        trace_id: target.trace_id,
        trace_ref: target.trace_ref,
    };
    assert!(store.for_trace(&admin, &bound).await.unwrap().is_some());
    assert!(store.for_trace(&beta, &bound).await.unwrap().is_none());
    let write = FeedbackWrite {
        trace: bound,
        author: "reviewer".into(),
        score: 5,
        comment: String::new(),
        at: Utc::now(),
    };
    assert!(store.upsert(&beta, &write).await.unwrap().is_none());
    assert!(store.upsert(&alpha, &write).await.unwrap().is_some());
}

#[rstest]
#[case::authorized("feedback-key", 200)]
#[case::other_key("other-key", 404)]
#[tokio::test]
async fn key_scoped_writers_only_reach_their_unassigned_traces(
    #[future(awt)] database: Database,
    #[case] key: &str,
    #[case] status: u16,
) {
    database.trace(TRACE, "", "feedback-key").await;
    let server = database.serve().await;
    let token = delegated(Identity {
        user_role: Role::InternalUser,
        user_id: None,
        token: Some(key.into()),
        ..Identity::default()
    });
    let response = server
        .client
        .put(server.url.join("/lens/feedback").unwrap())
        .bearer_auth(token)
        .json(&json!({"trace_id":TRACE,"score":8}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    if status == 200 {
        assert_eq!(response.json::<Value>().await.unwrap()["author"], key);
    }
}

#[rstest]
#[case::put_without_origin("PUT", None)]
#[case::delete_foreign_origin("DELETE", Some("https://other.test"))]
#[tokio::test]
async fn cookie_feedback_writes_require_the_lens_origin(
    #[future(awt)] database: Database,
    #[case] method: &str,
    #[case] origin: Option<&str>,
) {
    database.trace(TRACE, "alpha", "feedback-key").await;
    let server = database.serve().await;
    let login = server
        .client
        .post(server.url.join("/auth/session").unwrap())
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
        .request(
            method.parse().unwrap(),
            server
                .url
                .join(&format!("/lens/feedback?trace_id={TRACE}"))
                .unwrap(),
        )
        .header("cookie", cookie)
        .json(&json!({"trace_id":TRACE,"score":5}));
    let request = if let Some(origin) = origin {
        request.header("origin", origin)
    } else {
        request
    };
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), 403);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":"Lens session requests must come from the Lens origin"})
    );
}

#[rstest]
#[case::feedback("/lens/feedback/?trace_id=trace", "/lens/feedback?trace_id=trace")]
#[case::summary("/lens/feedback/summary/", "/lens/feedback/summary")]
#[tokio::test]
async fn feedback_trailing_slashes_keep_the_existing_redirect(
    #[future(awt)] database: Database,
    #[case] path: &str,
    #[case] target: &str,
) {
    let server = database.serve().await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client
        .get(server.url.join(path).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 307);
    assert_eq!(
        response.headers()["location"],
        server.url.join(target).unwrap().as_str()
    );
}

#[rstest]
#[tokio::test]
async fn unavailable_storage_returns_an_error_without_exposing_internal_state(
    #[future(awt)] database: Database,
) {
    database
        .state
        .schema_ready
        .store(false, std::sync::atomic::Ordering::Release);
    let server = database.serve().await;
    let response = server
        .client
        .get(server.url.join("/lens/feedback").unwrap())
        .query(&[("trace_id", TRACE)])
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":"Lens feedback is temporarily unavailable"})
    );
}

#[rstest]
#[tokio::test]
async fn summaries_accept_the_full_documented_batch_size(#[future(awt)] database: Database) {
    let server = database.serve().await;
    let response = server
        .client
        .post(server.url.join("/lens/feedback/summary").unwrap())
        .bearer_auth(ADMIN)
        .json(&json!({"traces":vec![json!({"trace_id":"missing"});500]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let rows: Vec<Value> = response.json().await.unwrap();
    assert_eq!(rows.len(), 500);
    assert_eq!(
        rows[0],
        json!({"trace_id":"missing","trace_ref":"","count":0,"average":null,"lowest":null})
    );
    assert_eq!(rows.last(), rows.first());
}
