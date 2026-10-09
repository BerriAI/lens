#[path = "sessions/support.rs"]
pub mod support;

use std::path::PathBuf;

use lens_contract::datasets::Dataset;
use lens_datasets::{DatasetReader, Finding, ReadError, Scope};
use lens_server::datasets::DatasetConfig;
use litellm_storage_clickhouse::datasets::Datasets;
use litellm_traces::{SpanDetail, Trace};
use rstest::rstest;
use serde_json::{Value, json};
use support::{ADMIN, Database, SECRET, Server, database};

#[derive(Clone, Copy)]
enum SourceResult {
    Missing,
    Invalid,
    Changed,
    TooLarge,
    Unavailable,
    LensMissing,
}

impl SourceResult {
    fn result<T>(self) -> Result<Option<T>, ReadError> {
        match self {
            Self::Missing => Ok(None),
            Self::Invalid => Err(ReadError::InvalidRequest(Box::new(std::io::Error::other(
                "invalid trace reference",
            )))),
            Self::Changed => Err(ReadError::TraceChanged(Box::new(std::io::Error::other(
                "Trace changed while reading",
            )))),
            Self::TooLarge => Err(ReadError::TooLarge),
            Self::Unavailable => Err(ReadError::Unavailable(Box::new(std::io::Error::other(
                "private source error",
            )))),
            Self::LensMissing => Err(ReadError::LensNotFound),
        }
    }
}

impl DatasetReader for SourceResult {
    async fn trace(&self, _: &str, _: &str) -> Result<Option<Trace>, ReadError> {
        self.result()
    }
    async fn span(&self, _: &str, _: &str, _: &str) -> Result<Option<SpanDetail>, ReadError> {
        self.result()
    }
    async fn findings(&self, _: &str, _: &[String], _: &Scope) -> Result<Vec<Finding>, ReadError> {
        self.result().map(Option::unwrap_or_default)
    }
}

async fn serve(database: &Database, source: SourceResult, config: DatasetConfig) -> Server {
    let datasets = Datasets(database.store.clone());
    database
        .serve_router(false, |auth| {
            lens_server::sessions::router_with_auth(auth.clone()).merge(
                lens_server::datasets::router(auth, datasets, source, config),
            )
        })
        .await
}

#[rstest]
#[tokio::test]
async fn python_dataset_fixtures_replay_over_real_http_and_clickhouse(
    #[future(awt)] database: Database,
) {
    let server = serve(&database, SourceResult::Missing, DatasetConfig::default()).await;
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../parity/fixtures/datasets");
    let report = lens_parity::replay_fixtures(
        &server.url,
        &fixtures,
        &lens_parity::Tokens::new(ADMIN, SECRET),
    )
    .await
    .unwrap();
    assert!(report.failures.is_empty(), "{:#?}", report.failures);
    assert_eq!(report.passed, 25);
}

#[rstest]
#[tokio::test]
async fn competing_revision_writes_have_one_winner_and_survive_restart(
    #[future(awt)] database: Database,
) {
    let server = serve(&database, SourceResult::Missing, DatasetConfig::default()).await;
    let created = server
        .client
        .post(server.url.join("/lens/datasets").unwrap())
        .bearer_auth(ADMIN)
        .json(&json!({"name":"concurrent revision"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200);
    let original = created.json::<Dataset>().await.unwrap();
    let revisions = server
        .url
        .join(&format!("/lens/datasets/{}/revisions", original.id))
        .unwrap();
    let first = server.client.post(revisions.clone()).bearer_auth(ADMIN).json(&json!({"base_revision":0,"cases":[
        {"id":"client-id", "messages":[{"role":"user","content":"one winner"}], "source":{}, "expected":"keep me"},
        {"id":"duplicate-client-id", "messages":[{"role":"user","content":"one winner"}], "source":{}, "expected":"discard me"}
    ]})).send();
    let second = server
        .client
        .post(revisions)
        .bearer_auth(ADMIN)
        .json(&json!({"base_revision":0,"cases":[]}))
        .send();
    let (first, second) = tokio::join!(first, second);
    let first = first.unwrap();
    let second = second.unwrap();
    assert!(matches!(
        (first.status().as_u16(), second.status().as_u16()),
        (200, 409) | (409, 200)
    ));
    let (winner, loser) = if first.status() == 200 {
        (first, second)
    } else {
        (second, first)
    };
    assert_eq!(
        loser.json::<Value>().await.unwrap(),
        json!({"detail":"Dataset changed, reload"})
    );
    let saved = winner.json::<Dataset>().await.unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.created_at, original.created_at);
    assert_eq!(saved.created_by, "lens-admin");
    if let Some(case) = saved.cases.first() {
        assert_eq!(saved.cases.len(), 1);
        assert_eq!(case.expected, "keep me");
        assert_ne!(case.id, "client-id");
    }
    drop(server);
    let server = serve(&database, SourceResult::Missing, DatasetConfig::default()).await;
    let endpoint = server
        .url
        .join(&format!("/lens/datasets/{}", original.id))
        .unwrap();
    let latest = server
        .client
        .get(endpoint.clone())
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(latest.status(), 200);
    assert_eq!(latest.json::<Dataset>().await.unwrap(), saved);
    let oldest = server
        .client
        .get(endpoint)
        .query(&[("revision", 0)])
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(oldest.status(), 200);
    assert_eq!(oldest.json::<Dataset>().await.unwrap(), original);
}

#[rstest]
#[case::missing(None)]
#[case::foreign(Some("https://other.test"))]
#[tokio::test]
async fn dataset_cookie_writes_require_lens_origin(
    #[future(awt)] database: Database,
    #[case] origin: Option<&str>,
) {
    let server = serve(&database, SourceResult::Missing, DatasetConfig::default()).await;
    let login = server
        .client
        .post(server.endpoint())
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
        .post(server.url.join("/lens/datasets").unwrap())
        .header("cookie", cookie)
        .json(&json!({"name":"browser dataset"}));
    let request = if let Some(origin) = origin {
        request.header("origin", origin)
    } else {
        request
    };
    let denied = request.send().await.unwrap();
    assert_eq!(denied.status(), 403);
    assert_eq!(
        denied.json::<Value>().await.unwrap(),
        json!({"detail":"Lens session requests must come from the Lens origin"})
    );
    let created = server
        .client
        .post(server.url.join("/lens/datasets").unwrap())
        .header("cookie", cookie)
        .header("origin", server.url.origin().ascii_serialization())
        .json(&json!({"name":"browser dataset"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200);
    let dataset = created.json::<Dataset>().await.unwrap();
    let read = server
        .client
        .get(
            server
                .url
                .join(&format!("/lens/datasets/{}", dataset.id))
                .unwrap(),
        )
        .header("cookie", cookie)
        .header("origin", "https://other.test")
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), 200);
    assert_eq!(read.json::<Dataset>().await.unwrap(), dataset);
}

#[rstest]
#[case::missing(None, "Sign in to Lens")]
#[case::invalid(Some("Bearer wrong"), "Invalid or expired gateway identity")]
#[tokio::test]
async fn dataset_authentication_rejects_untrusted_credentials(
    #[future(awt)] database: Database,
    #[case] authorization: Option<&str>,
    #[case] detail: &str,
) {
    let server = serve(&database, SourceResult::Missing, DatasetConfig::default()).await;
    let request = server
        .client
        .get(server.url.join("/lens/datasets").unwrap());
    let request = if let Some(authorization) = authorization {
        request.header("authorization", authorization)
    } else {
        request
    };
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":detail})
    );
}

#[rstest]
#[case::invalid(SourceResult::Invalid, 400, json!({"code":"invalid_request","message":"invalid trace reference"}), None)]
#[case::changed(SourceResult::Changed, 409, json!({"code":"trace_changed","message":"Trace changed while reading"}), None)]
#[case::too_large(SourceResult::TooLarge, 413, json!({"code":"too_large","message":"Trace is too large for this view. Use a filtered trace query."}), None)]
#[case::unavailable(SourceResult::Unavailable, 503, json!({"code":"unavailable","message":"Traces are temporarily unavailable. Please try again."}), Some("7"))]
#[case::lens_missing(SourceResult::LensMissing, 404, json!("Lens not found"), None)]
#[tokio::test]
async fn dataset_source_failures_preserve_http_error_contract(
    #[future(awt)] database: Database,
    #[case] source: SourceResult,
    #[case] status: u16,
    #[case] detail: Value,
    #[case] retry: Option<&str>,
) {
    let server = serve(
        &database,
        source,
        DatasetConfig {
            trace_retry_after_seconds: 7,
            ..DatasetConfig::default()
        },
    )
    .await;
    let request_source = if matches!(source, SourceResult::LensMissing) {
        json!({"kind":"finding","lens_id":"missing","finding_ids":["f1"]})
    } else {
        json!({"kind":"trace","trace_id":"trace"})
    };
    let response = server
        .client
        .post(server.url.join("/lens/datasets/build").unwrap())
        .bearer_auth(ADMIN)
        .json(&json!({"sources":[request_source]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    assert_eq!(
        response
            .headers()
            .get("retry-after")
            .map(|value| value.to_str().unwrap()),
        retry
    );
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":detail})
    );
}

#[rstest]
#[tokio::test]
async fn dataset_requests_are_not_rejected_by_axums_default_two_megabyte_limit(
    #[future(awt)] database: Database,
) {
    let server = serve(&database, SourceResult::Missing, DatasetConfig::default()).await;
    let body = format!(
        "{}{{\"name\":\"large request\"}}",
        " ".repeat(2 * 1024 * 1024)
    );
    let response = server
        .client
        .post(server.url.join("/lens/datasets").unwrap())
        .bearer_auth(ADMIN)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Dataset>().await.unwrap().name,
        "large request"
    );
}

#[rstest]
#[case::malformed("{",422,json!({"detail":[{"type":"json_invalid","loc":["body",1],"msg":"JSON decode error","input":{},"ctx":{"error":"Expecting property name enclosed in double quotes"}}]}))]
#[case::valid_json_invalid_schema("{}",401,json!({"detail":"Sign in to Lens"}))]
#[tokio::test]
async fn malformed_json_precedes_authentication_but_schema_validation_follows_it(
    #[future(awt)] database: Database,
    #[case] body: &str,
    #[case] status: u16,
    #[case] expected: Value,
) {
    let server = serve(&database, SourceResult::Missing, DatasetConfig::default()).await;
    let response = server
        .client
        .post(server.url.join("/lens/datasets").unwrap())
        .header("content-type", "application/json")
        .body(body.to_owned())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    assert_eq!(response.json::<Value>().await.unwrap(), expected);
}

#[rstest]
#[case::list("/lens/datasets/?revision=0", "/lens/datasets?revision=0")]
#[case::multiple("/lens/datasets///", "/lens/datasets")]
#[case::read("/lens/datasets/example/", "/lens/datasets/example")]
#[case::build("/lens/datasets/build/", "/lens/datasets/build")]
#[case::export(
    "/lens/datasets/example/export/?revision=2",
    "/lens/datasets/example/export?revision=2"
)]
#[tokio::test]
async fn trailing_slashes_preserve_fastapi_redirects(
    #[future(awt)] database: Database,
    #[case] path: &str,
    #[case] target: &str,
) {
    let server = serve(&database, SourceResult::Missing, DatasetConfig::default()).await;
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
    assert!(response.bytes().await.unwrap().is_empty());
}

#[rstest]
#[case::number("1000000000000000000000000000000")]
#[case::string("\"1000000000000000000000000000000\"")]
#[case::float("1e30")]
#[tokio::test]
async fn revisions_outside_rust_integer_range_preserve_lookup_and_conflict_behavior(
    #[future(awt)] database: Database,
    #[case] revision: &str,
) {
    let server = serve(&database, SourceResult::Missing, DatasetConfig::default()).await;
    let created = server
        .client
        .post(server.url.join("/lens/datasets").unwrap())
        .bearer_auth(ADMIN)
        .json(&json!({"name":"wide revisions"}))
        .send()
        .await
        .unwrap()
        .json::<Dataset>()
        .await
        .unwrap();
    let path = format!("/lens/datasets/{}", created.id);
    let read = server
        .client
        .get(server.url.join(&path).unwrap())
        .bearer_auth(ADMIN)
        .query(&[("revision", "1000000000000000000000000000000")])
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), 404);
    assert_eq!(
        read.json::<Value>().await.unwrap(),
        json!({"detail":"Dataset not found"})
    );
    let save = server
        .client
        .post(server.url.join(&format!("{path}/revisions")).unwrap())
        .bearer_auth(ADMIN)
        .header("content-type", "application/json")
        .body(format!("{{\"base_revision\":{revision},\"cases\":[]}}"))
        .send()
        .await
        .unwrap();
    assert_eq!(save.status(), 409);
    assert_eq!(
        save.json::<Value>().await.unwrap(),
        json!({"detail":"Dataset changed, reload"})
    );
    let missing = server
        .client
        .post(server.url.join("/lens/datasets/missing/revisions").unwrap())
        .bearer_auth(ADMIN)
        .header("content-type", "application/json")
        .body(format!("{{\"base_revision\":{revision},\"cases\":[]}}"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}
