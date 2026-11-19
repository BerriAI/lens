use litellm_lens::{Error, Storage, config::http_client};
use rstest::rstest;
use std::time::Duration;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};

fn storage(address: std::net::SocketAddr) -> Storage {
    let config =
        litellm_traces_clickhouse::Config::new("lens".into(), &format!("http://{address}"), 14)
            .unwrap();
    Storage::new(config, http_client().unwrap(), "query-secret".into())
}

fn unused_address() -> std::net::SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

#[rstest]
#[case::answering_queries(200)]
#[case::answering_with_an_error(500)]
#[tokio::test]
async fn startup_waits_for_clickhouse_that_starts_listening_late(#[case] status: u16) {
    let address = unused_address();
    let storage = storage(address);
    let late = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let server = MockServer::builder()
            .listener(std::net::TcpListener::bind(address).unwrap())
            .start()
            .await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        server
    });

    let outcome = storage
        .reachable(Duration::from_secs(30), Duration::from_millis(50))
        .await;

    let server = late.await.unwrap();
    assert!(outcome.is_ok());
    assert!(!server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn startup_reports_the_transport_failure_once_patience_runs_out() {
    let storage = storage(unused_address());

    let outcome = storage
        .reachable(Duration::from_millis(200), Duration::from_millis(50))
        .await;

    assert!(matches!(
        outcome,
        Err(Error::Storage(litellm_traces_clickhouse::Error::Storage(
            litellm_storage_clickhouse::Error::Transport
        )))
    ));
}
