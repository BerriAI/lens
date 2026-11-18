use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region, retry::RetryConfig};
use chrono::{TimeZone, Utc};
use lens_payloads::{Error, Field, ObjectKey, Store, sha256_hex};
use rstest::{fixture, rstest};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
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

fn listing(prefix: &str, keys: &[&str]) -> String {
    let contents: String = keys
        .iter()
        .map(|key| format!("<Contents><Key>{key}</Key><Size>1</Size></Contents>"))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>{BUCKET}</Name><Prefix>{prefix}</Prefix><KeyCount>{}</KeyCount><MaxKeys>1000</MaxKeys><IsTruncated>false</IsTruncated>{contents}</ListBucketResult>",
        keys.len()
    )
}

#[rstest]
#[tokio::test]
async fn sweep_deletes_only_days_past_retention(#[future(awt)] server: MockServer) {
    let expired = "v1/2026/01/02/";
    Mock::given(method("GET"))
        .and(path(format!("/{BUCKET}/")))
        .and(query_param("prefix", expired))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(listing(expired, &["v1/2026/01/02/abc/evt_1/messages"])),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{BUCKET}/")))
        .respond_with(ResponseTemplate::new(200).set_body_string(listing("", &[])))
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/{BUCKET}/")))
        .and(query_param("delete", ""))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><DeleteResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"></DeleteResult>",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let today = Utc.with_ymd_and_hms(2026, 1, 17, 12, 0, 0).unwrap();
    let deleted = store(&server).sweep(today, 14).await.unwrap();

    assert_eq!(deleted, 1);
    let listed: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.method.as_str() == "GET")
        .filter_map(|request| {
            request
                .url
                .query_pairs()
                .find(|(name, _)| name == "prefix")
                .map(|(_, value)| value.into_owned())
        })
        .collect();
    assert!(
        !listed
            .iter()
            .any(|prefix| prefix.as_str() >= "v1/2026/01/03/")
    );
    assert!(listed.contains(&expired.to_owned()));
}
