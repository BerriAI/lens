use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{process::Child, sync::mpsc};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

use super::{Config, Prompt, command, emit, terminate, write_owned};
use crate::Error;

pub(super) struct Gateway {
    child: Child,
    client: Client,
    base: String,
    config: Config,
    root: PathBuf,
}

#[derive(Clone)]
pub(super) struct History {
    client: Client,
    base: String,
    token: String,
}

impl History {
    pub async fn read(&self, session_id: &str) -> Result<Value, Error> {
        if session_id.is_empty()
            || !session_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(Error::Invalid("Invalid native session"));
        }
        let response = self
            .client
            .get(format!("{}/api/sessions/{session_id}/messages", self.base))
            .query(&[
                ("limit", "500"),
                ("order", "latest"),
                ("include_compacted", "true"),
            ])
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|_| Error::Runtime)?;
        let body: Value = response
            .error_for_status()
            .map_err(|_| Error::Runtime)?
            .json()
            .await
            .map_err(|_| Error::Runtime)?;
        let messages = body["data"].as_array().ok_or(Error::Runtime)?.iter().filter(|m| matches!(m["role"].as_str(),Some("user"|"assistant")) && m["content"].as_str().is_some_and(|t|!t.is_empty()))
            .map(|m|json!({"id":m["id"],"role":m["role"],"content":m["content"],"created_at":m["created_at"]})).collect::<Vec<_>>();
        Ok(
            json!({"messages":messages,"truncated":body["pagination"]["returned"].as_u64()==Some(500)}),
        )
    }
}

impl Gateway {
    pub fn history(&self) -> History {
        History {
            client: self.client.clone(),
            base: self.base.clone(),
            token: self.config.token.clone(),
        }
    }

    async fn session(&self, prompt: &Prompt) -> Result<String, Error> {
        if let Some(id) = &prompt.native_id {
            return Ok(id.clone());
        }
        let existing = if prompt.permanent {
            let response = self
                .client
                .get(format!("{}/api/sessions", self.base))
                .query(&[("title", "Bot Chat")])
                .bearer_auth(&self.config.token)
                .send()
                .await
                .map_err(|_| Error::Runtime)?;
            let body: Value = response
                .error_for_status()
                .map_err(|_| Error::Runtime)?
                .json()
                .await
                .map_err(|_| Error::Runtime)?;
            body["data"]
                .as_array()
                .and_then(|rows| rows.iter().find(|row| row["title"] == "Bot Chat"))
                .and_then(|row| row["id"].as_str())
                .map(str::to_owned)
        } else {
            None
        };
        if let Some(id) = existing {
            return Ok(id);
        }
        let body = if prompt.permanent {
            json!({"title":"Bot Chat","source":"api_server"})
        } else {
            json!({"id":prompt.conversation_id,"title":prompt.conversation_id,"source":"api_server"})
        };
        let response = self
            .client
            .post(format!("{}/api/sessions", self.base))
            .bearer_auth(&self.config.token)
            .json(&body)
            .send()
            .await
            .map_err(|_| Error::Runtime)?;
        if response.status() == reqwest::StatusCode::CONFLICT && !prompt.permanent {
            return Ok(prompt.conversation_id.clone());
        }
        let body: Value = response
            .error_for_status()
            .map_err(|_| Error::Runtime)?
            .json()
            .await
            .map_err(|_| Error::Runtime)?;
        body.pointer("/session/id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(Error::Runtime)
    }

    pub async fn start(config: &Config, root: &Path) -> Result<Self, Error> {
        let executable = std::env::var("LENS_HERMES_EXECUTABLE")
            .unwrap_or_else(|_| "/opt/hermes/.venv/bin/hermes".into());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        drop(listener);
        let settings = json!({
            "_config_version":54,
            "model":{"default":config.model,"provider":"custom","base_url":"${OPENAI_BASE_URL}","api_key":"${OPENAI_API_KEY}"},
            "terminal":{"backend":"local","cwd":root.join("work")},
            "approvals":{"mode":"off"},
            "mcp_servers":{"lens":{"url":"${LENS_TOOLS_URL}","headers":{"Authorization":"Bearer ${LENS_AGENT_TOKEN}"},"trust":"full"}}
        });
        write_owned(
            &root.join("profile/config.yaml"),
            &serde_json::to_string(&settings)?,
            config.uid,
        )
        .await?;
        write_owned(
            &root.join("profile/SOUL.md"),
            &config.instructions,
            config.uid,
        )
        .await?;
        let child = command(&executable, config, root)
            .env("HOME", root.join("work"))
            .env("HERMES_HOME", root.join("profile"))
            .env("HERMES_MODEL", &config.model)
            .env("HERMES_INFERENCE_PROVIDER", "custom")
            .env("API_SERVER_ENABLED", "true")
            .env("API_SERVER_HOST", "127.0.0.1")
            .env("API_SERVER_PORT", port.to_string())
            .env("API_SERVER_KEY", &config.token)
            .args(["gateway", "run", "--no-supervise", "--external-supervisor"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()?;
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::Runtime)?;
        let gateway = Self {
            child,
            client,
            base: format!("http://127.0.0.1:{port}"),
            config: config.clone(),
            root: root.into(),
        };
        for _ in 0..120 {
            if gateway
                .client
                .get(format!("{}/health", gateway.base))
                .bearer_auth(&config.token)
                .timeout(Duration::from_secs(1))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                return Ok(gateway);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Err(Error::Runtime)
    }

    pub async fn prompt(
        &mut self,
        prompt: &Prompt,
        events: &mpsc::Sender<Result<String, std::io::Error>>,
    ) -> Result<(), Error> {
        let native_id = self.session(prompt).await?;
        if prompt.permanent && prompt.native_id.is_none() {
            self.dispatch(&native_id, "heartbeat", "every 10m Continue your standing task using the latest instructions and corrections in this chat. Report meaningful results; remain quiet when nothing needs attention.").await?;
            emit(
                events,
                "bot_status",
                json!({"heartbeat":"active","interval":"10m"}),
            )
            .await?;
        }
        emit(events, "session", json!({"native_id":native_id})).await?;
        let response = self
            .client
            .post(format!(
                "{}/api/sessions/{native_id}/chat/stream",
                self.base
            ))
            .bearer_auth(&self.config.token)
            .header("Idempotency-Key", &prompt.turn_id)
            .json(&json!({"input":prompt.text}))
            .send()
            .await
            .map_err(|_| Error::Runtime)?;
        if !response.status().is_success() {
            return Err(Error::Runtime);
        }
        let mut stream = response.bytes_stream();
        let mut buffer = Vec::new();
        let mut terminal = false;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| Error::Runtime)?;
            buffer.extend_from_slice(&chunk);
            while let Some(end) = buffer.windows(2).position(|part| part == b"\n\n") {
                let frame = buffer.drain(..end + 2).collect::<Vec<_>>();
                let frame = std::str::from_utf8(&frame).map_err(|_| Error::Runtime)?;
                let kind = frame
                    .lines()
                    .find_map(|l| l.strip_prefix("event:"))
                    .map(str::trim)
                    .unwrap_or("message");
                let data = frame
                    .lines()
                    .filter_map(|l| l.strip_prefix("data:"))
                    .collect::<Vec<_>>()
                    .join("\n");
                if let Ok(data) = serde_json::from_str(&data) {
                    let kind = match kind {
                        "run.completed" => {
                            terminal = true;
                            "completed"
                        }
                        "run.failed" => {
                            terminal = true;
                            "failed"
                        }
                        "run.cancelled" => {
                            terminal = true;
                            "interrupted"
                        }
                        other => other,
                    };
                    emit(events, kind, data).await?;
                }
            }
        }
        if !terminal {
            return Err(Error::Runtime);
        }
        Ok(())
    }

    async fn dispatch(&self, session_id: &str, name: &str, arg: &str) -> Result<(), Error> {
        let executable = std::env::var("LENS_HERMES_EXECUTABLE")
            .unwrap_or_else(|_| "/opt/hermes/.venv/bin/hermes".into());
        let attach = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let output = command(&executable, &self.config, &self.root)
                    .env("HOME", self.root.join("work"))
                    .env("HERMES_HOME", self.root.join("profile"))
                    .args(["gateway", "attach", "--json"])
                    .output()
                    .await?;
                let attach: Value = serde_json::from_slice(&output.stdout)
                    .map_err(|_| Error::Invalid("Hermes attachment did not return JSON"))?;
                if output.status.success() {
                    return Ok::<Value, Error>(attach);
                }
                if !matches!(attach["state"].as_str(), Some("starting" | "restart_wait")) {
                    return Err(Error::Invalid("Hermes gateway attachment failed"));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
        .await
        .map_err(|_| Error::Invalid("Hermes control did not become ready"))??;
        let url = attach["url"]
            .as_str()
            .ok_or(Error::Invalid("Hermes attachment has no endpoint"))?;
        let mut request = url.into_client_request().map_err(|_| Error::Runtime)?;
        let protocols = attach["protocols"]
            .as_array()
            .ok_or(Error::Runtime)?
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        request.headers_mut().insert(
            "sec-websocket-protocol",
            protocols.parse().map_err(|_| Error::Runtime)?,
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|_| Error::Invalid("Hermes control connection failed"))?;
        socket.send(Message::Text(json!({"jsonrpc":"2.0","id":1,"method":"command.dispatch","params":{"session_id":session_id,"name":name,"arg":arg}}).to_string())).await.map_err(|_|Error::Runtime)?;
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            while let Some(message) = socket.next().await {
                let message = message.map_err(|_| Error::Runtime)?;
                if let Message::Text(text) = message {
                    let value: Value = serde_json::from_str(&text)?;
                    if value.get("id") == Some(&json!(1)) {
                        return if value.get("error").is_some() {
                            match value.pointer("/error/data/reason").and_then(Value::as_str) {
                                Some("permission_denied") => {
                                    Err(Error::Invalid("Hermes refused session control permission"))
                                }
                                Some("not_found") => Err(Error::Invalid(
                                    "Hermes could not find the session for control",
                                )),
                                Some("invalid_params") => {
                                    Err(Error::Invalid("Hermes refused control arguments"))
                                }
                                _ => Err(Error::Invalid("Hermes refused heartbeat setup")),
                            }
                        } else {
                            Ok(())
                        };
                    }
                }
            }
            Err(Error::Runtime)
        })
        .await
        .map_err(|_| Error::Runtime)?;
        let _ = socket.close(None).await;
        result
    }

    pub fn alive(&mut self) -> bool {
        self.child.try_wait().is_ok_and(|status| status.is_none())
    }

    pub async fn stop(&mut self) -> Result<(), Error> {
        terminate(&mut self.child).await
    }
}
