use std::time::Duration;

use litellm_http::Client;
use serde::{Serialize, de::DeserializeOwned};

use crate::{Connection, Error, Parameter};

pub(super) async fn command(
    client: &Client,
    connection: &Connection,
    query: &'static str,
    parameters: &[(&str, String)],
    body: String,
) -> Result<String, Error> {
    let mut url = connection.url().clone();
    let pairs: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| {
            !key.starts_with("param_")
                && !matches!(
                    key.as_ref(),
                    "query"
                        | "keeper_map_strict_mode"
                        | "insert_keeper_max_retries"
                        | "async_insert"
                        | "wait_end_of_query"
                        | "send_progress_in_http_headers"
                        | "output_format_json_quote_64bit_integers"
                )
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.query_pairs_mut()
        .clear()
        .extend_pairs(pairs)
        .append_pair("query", query)
        .append_pair("keeper_map_strict_mode", "1")
        .append_pair("insert_keeper_max_retries", "0")
        .append_pair("async_insert", "0")
        .append_pair("wait_end_of_query", "1")
        .append_pair("send_progress_in_http_headers", "0")
        .append_pair("output_format_json_quote_64bit_integers", "0")
        .extend_pairs(parameters.iter().map(|(key, value)| {
            (
                format!("param_{key}"),
                Parameter::Text(value.clone()).encoded(),
            )
        }));
    let mut response = client
        .post(url)
        .timeout(Duration::from_secs(30))
        .header("Content-Length", body.len().to_string())
        .body(body)
        .send()
        .await
        .map_err(|_| Error::Transport)?;
    let status = response.status();
    let code = response
        .headers()
        .get("x-clickhouse-exception-code")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Transport)? {
        bytes.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(bytes).map_err(|_| Error::InvalidResponse)?;
    if !status.is_success() || code.is_some_and(|value| value != 0) {
        return Err(failure(status.as_u16(), code, &text));
    }
    Ok(text)
}

fn failure(status: u16, code: Option<u32>, text: &str) -> Error {
    let code = code.or_else(|| text.strip_prefix("Code: ")?.split('.').next()?.parse().ok());
    match code {
        Some(395) if text.contains("LENS_STATE_CONFLICT") => Error::StateConflict,
        Some(999) if text.to_ascii_lowercase().contains("(node exists)") => Error::StateExists,
        Some(999) if text.to_ascii_lowercase().contains("bad version") => Error::StateConflict,
        _ => Error::StateFailed { status, code },
    }
}

pub(super) fn encode(value: &impl Serialize) -> Result<String, Error> {
    serde_json::to_string(value).map_err(|_| Error::InvalidState)
}

pub(super) fn decode<T: DeserializeOwned>(body: &str) -> Result<Vec<T>, Error> {
    body.lines()
        .map(|line| serde_json::from_str(line).map_err(|_| Error::InvalidResponse))
        .collect()
}
