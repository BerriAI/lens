use litellm_http::Client;
use litellm_storage_clickhouse::{Connection, execute_statement};
use rstest::fixture;
use serde_json::{Value, json};
use std::time::Duration;
use testcontainers_modules::{
    clickhouse::ClickHouse,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};

pub struct Database {
    _container: Option<ContainerAsync<ClickHouse>>,
    pub client: Client,
    pub reader: Connection,
    pub writer: Connection,
    name: String,
}

impl Drop for Database {
    fn drop(&mut self) {
        if self._container.is_some() {
            return;
        }
        let connection = self.writer.clone();
        let name = self.name.clone();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    execute_statement(
                        &Client::no_redirect_for_test(),
                        &connection,
                        &format!("DROP DATABASE `{name}` SYNC"),
                        Duration::from_secs(30),
                    )
                    .await
                    .unwrap();
                });
        })
        .join()
        .unwrap();
    }
}

#[fixture]
pub async fn database() -> Database {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let (container, url) = match std::env::var("CLICKHOUSE_STATE_TEST_URL") {
        Ok(url) => (None, url),
        Err(_) => {
            let container = ClickHouse::default().with_tag("26.9.6.6@sha256:eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e").with_env_var("CLICKHOUSE_SKIP_USER_SETUP", "1").start().await.unwrap();
            let url = format!(
                "http://{}:{}",
                container.get_host().await.unwrap(),
                container.get_host_port_ipv4(8123).await.unwrap()
            );
            (Some(container), url)
        }
    };
    let name = format!(
        "lens_eval_query_{}_{}_{}",
        std::process::id(),
        time::OffsetDateTime::now_utc().unix_timestamp_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let writer = Connection::writer(&url).unwrap();
    let client = Client::no_redirect_for_test();
    litellm_traces_clickhouse::ensure_schema(&client, &writer, &name, 7)
        .await
        .unwrap();
    let reader = Connection::reader(&url, &name).unwrap();
    Database {
        _container: container,
        client,
        reader,
        writer,
        name,
    }
}

impl Database {
    pub async fn insert(&self, rows: Vec<Value>) {
        let content = rows
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        execute_statement(
            &self.client,
            &self.writer,
            &format!(
                "INSERT INTO `{}`.otel_traces FORMAT JSONEachRow\n{content}",
                self.name
            ),
            Duration::from_secs(30),
        )
        .await
        .unwrap();
    }
}

pub fn span(team: &str, trace: &str, id: &str, session: &str, received: u64) -> Value {
    let timestamp = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    json!({
        "Timestamp":timestamp,"TraceId":trace,"SpanId":id,"SpanName":id,"TeamId":team,"ApiKeyHash":"key",
        "Duration":1000000,"EngineReceivedMs":received,"StatusCode":"STATUS_CODE_OK",
        "SpanAttributes":{"session.id":session,"agent.version":"build"},"Input":"question","Output":"answer"
    })
}
