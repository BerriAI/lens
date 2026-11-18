use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region, retry::RetryConfig};
use chrono::{TimeZone, Utc};
use lens_payloads::{Error, Field, ObjectKey, Store, Sweep, sha256_hex};
use rstest::{fixture, rstest};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param, query_param_is_missing},
};

const BUCKET: &str = "lens";
const BODY: &str = "[{\"role\":\"user\",\"content\":\"雪 snow\"}]";

#[fixture]
async fn server() -> MockServer {
    MockServer::start().await
}

fn store(server: &MockServer) -> Store {
    let config = aws_sdk_s3::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new("AKID", "SECRET", None, None, "test"))
        .endpoint_url(server.uri())
        .force_path_style(true)
        .retry_config(RetryConfig::disabled())
        .build();
    Store::new(
        aws_sdk_s3::Client::from_conf(config),
        BUCKET.into(),
        String::new(),
    )
}

fn key() -> ObjectKey {
    ObjectKey::new(
        "",
        Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
        "team",
        "key",
        "evt_1",
        Field::Messages,
    )
    .unwrap()
}

fn object_path() -> String {
    format!("/{BUCKET}/{}", key().as_str())
}

#[rstest]
#[tokio::test]
async fn put_refuses_a_body_that_does_not_match_its_checksum(#[future(awt)] server: MockServer) {
    let result = store(&server)
        .put(&key(), BODY.as_bytes().to_vec(), &sha256_hex(b"other"))
        .await;

    assert!(matches!(result, Err(Error::Integrity)));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[rstest]
#[tokio::test]
async fn put_writes_the_tenant_scoped_key(#[future(awt)] server: MockServer) {
    Mock::given(method("PUT"))
        .and(path(object_path()))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    store(&server)
        .put(
            &key(),
            BODY.as_bytes().to_vec(),
            &sha256_hex(BODY.as_bytes()),
        )
        .await
        .unwrap();
}

#[rstest]
#[tokio::test]
async fn put_reports_store_failure(#[future(awt)] server: MockServer) {
    Mock::given(method("PUT"))
        .and(path(object_path()))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let result = store(&server)
        .put(
            &key(),
            BODY.as_bytes().to_vec(),
            &sha256_hex(BODY.as_bytes()),
        )
        .await;

    assert!(matches!(result, Err(Error::Request(_))));
}

#[rstest]
#[case::matching(BODY, Ok(BODY))]
#[case::changed("[{\"role\":\"user\",\"content\":\"rain\"}]", Err(()))]
#[tokio::test]
async fn get_returns_only_bytes_matching_the_recorded_checksum(
    #[future(awt)] server: MockServer,
    #[case] served: &str,
    #[case] expected: Result<&str, ()>,
) {
    Mock::given(method("GET"))
        .and(path(object_path()))
        .respond_with(ResponseTemplate::new(200).set_body_string(served))
        .mount(&server)
        .await;

    let result = store(&server)
        .get(&key(), BODY.len() as u64, &sha256_hex(BODY.as_bytes()))
        .await;

    match expected {
        Ok(body) => assert_eq!(result.unwrap(), body),
        Err(()) => assert!(matches!(result, Err(Error::Integrity))),
    }
}

#[rstest]
#[tokio::test]
async fn get_reports_a_deleted_object_as_missing(#[future(awt)] server: MockServer) {
    Mock::given(method("GET"))
        .and(path(object_path()))
        .respond_with(ResponseTemplate::new(404).set_body_string(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>NoSuchKey</Code><Message>missing</Message></Error>",
        ))
        .mount(&server)
        .await;

    let result = store(&server)
        .get(&key(), BODY.len() as u64, &sha256_hex(BODY.as_bytes()))
        .await;

    assert!(matches!(result, Err(Error::Missing)));
}

const PREFIX: &str = "lens/";
const NAMESPACE: &str = "lens/lens-payloads/v1/";
const DELETED: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><DeleteResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"></DeleteResult>";

fn prefixed_store(server: &MockServer) -> Store {
    let config = aws_sdk_s3::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new("AKID", "SECRET", None, None, "test"))
        .endpoint_url(server.uri())
        .force_path_style(true)
        .retry_config(RetryConfig::disabled())
        .build();
    Store::new(
        aws_sdk_s3::Client::from_conf(config),
        BUCKET.into(),
        PREFIX.into(),
    )
}

fn generated(year: i32, month: u32, day: u32, event_id: &str) -> String {
    ObjectKey::new(
        PREFIX,
        Utc.with_ymd_and_hms(year, month, day, 23, 59, 59).unwrap(),
        "team",
        "key",
        event_id,
        Field::Messages,
    )
    .unwrap()
    .as_str()
    .to_owned()
}

fn listing(keys: &[String], next: Option<&str>) -> String {
    let contents: String = keys
        .iter()
        .map(|key| format!("<Contents><Key>{key}</Key><Size>1</Size></Contents>"))
        .collect();
    let continuation = next.map_or(String::new(), |token| {
        format!("<NextContinuationToken>{token}</NextContinuationToken>")
    });
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>{BUCKET}</Name><Prefix>{NAMESPACE}</Prefix><KeyCount>{}</KeyCount><MaxKeys>1000</MaxKeys><IsTruncated>{}</IsTruncated>{continuation}{contents}</ListBucketResult>",
        keys.len(),
        next.is_some(),
    )
}

async fn list_page(server: &MockServer, token: Option<&str>, body: String) {
    let listed = Mock::given(method("GET"))
        .and(path(format!("/{BUCKET}/")))
        .and(query_param("prefix", NAMESPACE));
    let page = match token {
        Some(token) => listed.and(query_param("continuation-token", token)),
        None => listed.and(query_param_is_missing("continuation-token")),
    };
    page.respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(server)
        .await;
}

async fn delete_responds(server: &MockServer, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path(format!("/{BUCKET}/")))
        .and(query_param("delete", ""))
        .respond_with(response)
        .mount(server)
        .await;
}

async fn deleted_bodies(server: &MockServer) -> String {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.method.as_str() == "POST")
        .map(|request| String::from_utf8_lossy(&request.body).into_owned())
        .collect()
}

fn today() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 17, 12, 0, 0).unwrap()
}

#[rstest]
#[tokio::test]
async fn sweep_deletes_every_expired_generated_object_and_stops_at_retained_days(
    #[future(awt)] server: MockServer,
) {
    let after_downtime = generated(2025, 6, 1, "evt_old");
    let unrelated = format!("{NAMESPACE}2025/06/01/notes.txt");
    let expired = generated(2026, 1, 2, "evt_expired");
    let retained = generated(2026, 1, 3, "evt_retained");
    list_page(
        &server,
        None,
        listing(&[after_downtime.clone(), unrelated.clone()], Some("page-2")),
    )
    .await;
    list_page(
        &server,
        Some("page-2"),
        listing(&[expired.clone(), retained.clone()], Some("page-3")),
    )
    .await;
    Mock::given(method("GET"))
        .and(query_param("continuation-token", "page-3"))
        .respond_with(ResponseTemplate::new(200).set_body_string(listing(&[], None)))
        .expect(0)
        .mount(&server)
        .await;
    delete_responds(&server, ResponseTemplate::new(200).set_body_string(DELETED)).await;

    let sweep = prefixed_store(&server).sweep(today(), 14).await.unwrap();

    assert_eq!(
        sweep,
        Sweep {
            deleted: 2,
            failed: 0
        }
    );
    let deleted = deleted_bodies(&server).await;
    assert!(deleted.contains(&after_downtime));
    assert!(deleted.contains(&expired));
    assert!(!deleted.contains(&unrelated));
    assert!(!deleted.contains(&retained));
}

#[rstest]
#[tokio::test]
async fn sweep_lists_only_the_lens_namespace_under_the_configured_prefix(
    #[future(awt)] server: MockServer,
) {
    list_page(&server, None, listing(&[], None)).await;

    let sweep = prefixed_store(&server).sweep(today(), 14).await.unwrap();

    assert_eq!(sweep, Sweep::default());
    let prefixes: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .flat_map(|request| {
            request
                .url
                .query_pairs()
                .filter(|(name, _)| name == "prefix")
                .map(|(_, value)| value.into_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(prefixes, [NAMESPACE]);
}

#[rstest]
#[tokio::test]
async fn sweep_reports_objects_the_store_refused_to_delete(#[future(awt)] server: MockServer) {
    let refused = generated(2026, 1, 1, "evt_locked");
    let removed = generated(2026, 1, 2, "evt_removed");
    list_page(&server, None, listing(&[refused.clone(), removed], None)).await;
    delete_responds(
        &server,
        ResponseTemplate::new(200).set_body_string(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><DeleteResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Error><Key>{refused}</Key><Code>AccessDenied</Code><Message>denied</Message></Error></DeleteResult>"
        )),
    )
    .await;

    let sweep = prefixed_store(&server).sweep(today(), 14).await.unwrap();

    assert_eq!(
        sweep,
        Sweep {
            deleted: 1,
            failed: 1
        }
    );
}

#[rstest]
#[case::listing_unavailable(503, 200)]
#[case::delete_unavailable(200, 503)]
#[tokio::test]
async fn sweep_fails_when_the_store_is_unavailable(
    #[future(awt)] server: MockServer,
    #[case] list_status: u16,
    #[case] delete_status: u16,
) {
    Mock::given(method("GET"))
        .and(path(format!("/{BUCKET}/")))
        .respond_with(
            ResponseTemplate::new(list_status)
                .set_body_string(listing(&[generated(2026, 1, 2, "evt")], None)),
        )
        .mount(&server)
        .await;
    delete_responds(
        &server,
        ResponseTemplate::new(delete_status).set_body_string(DELETED),
    )
    .await;

    let result = prefixed_store(&server).sweep(today(), 14).await;

    assert!(matches!(result, Err(Error::Request(_))));
}
