use lens_evals_sdk::{
    Error,
    client::{Client, endpoint},
    model::*,
};
use rstest::rstest;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path, query_param},
};

#[rstest]
#[case::unauthorized(401, "unauthorized", 1)]
#[case::scope(403, "request_failed", 1)]
#[case::closed(409, "run_closed", 1)]
#[case::rate_limit(429, "rate_limit", 3)]
#[case::unavailable(503, "unavailable", 3)]
#[tokio::test]
async fn bounded_retries_and_typed_failures(
    #[case] status: u16,
    #[case] code: &str,
    #[case] attempts: u64,
) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/lens/evals/runs/test"))
        .and(header("Authorization", "Bearer key"))
        .and(header("X-Lens-Contract", "1"))
        .respond_with(ResponseTemplate::new(status).set_body_json(json!({"code":code})))
        .expect(attempts)
        .mount(&server)
        .await;
    let error = Client::new(&server.uri(), "key")
        .unwrap()
        .get("test", false)
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Api { status: actual, code: actual_code } if actual == status && actual_code == code)
    );
}

#[rstest]
#[case::redirect(302, "{}")]
#[case::html(200, "<html>not an API</html>")]
#[case::identity(200, r#"{"id":"other","name":"wrong","revision":1}"#)]
#[tokio::test]
async fn rejects_invalid_resolver_responses(#[case] status: u16, #[case] body: &str) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        Client::new(&server.uri(), "key")
            .unwrap()
            .resolve("demo@1")
            .await
            .is_err()
    );
}

#[rstest]
#[tokio::test]
async fn legacy_resolution_keeps_pinned_revision_and_escaped_path() {
    let server = MockServer::start().await;
    Mock::given(path("/prefix/lens/datasets/resolve"))
        .and(query_param("name", "demo"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(path("/prefix/lens/datasets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id":"data/one","name":"demo","revision":7,"agent_name":"agent","case_count":1}])),
        )
        .mount(&server)
        .await;
    Mock::given(path("/prefix/lens/datasets/data%2Fone/revisions/3/cases"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"dataset_id":"data/one","revision":3,"cases":[]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::new(&format!("{}/prefix", server.uri()), "key").unwrap();
    let dataset = client.resolve("demo@3").await.unwrap();
    assert_eq!(dataset.revision, 3);
    assert_eq!(client.cases(&dataset).await.unwrap().dataset_id, "data/one");
}

#[rstest]
#[case::wrong_dataset("other", 1)]
#[case::wrong_revision("demo", 2)]
#[tokio::test]
async fn rejects_wrong_download_identity(#[case] id: &str, #[case] revision: u64) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"dataset_id":id,"revision":revision,"cases":[]})),
        )
        .mount(&server)
        .await;
    let dataset = ResolvedDataset {
        id: "demo".into(),
        name: "demo".into(),
        revision: 1,
    };
    assert!(
        Client::new(&server.uri(), "key")
            .unwrap()
            .cases(&dataset)
            .await
            .is_err()
    );
}

#[rstest]
#[case::remote_http("http://example.com")]
#[case::credentials("https://user:password@example.com")]
#[case::query("https://example.com?token=key")]
#[case::fragment("https://example.com#fragment")]
fn rejects_unsafe_endpoints(#[case] value: &str) {
    assert!(endpoint(value).is_err());
}
