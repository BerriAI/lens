use super::{Config, Prompt, command, emit, terminate, write_owned};
use crate::Error;
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout},
    sync::mpsc,
};

pub(super) struct Process {
    child: Child,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    loaded: HashSet<String>,
}

impl Process {
    pub async fn start(config: &Config, root: &Path) -> Result<Self, Error> {
        let executable = std::env::var("LENS_DEEPAGENTS_EXECUTABLE")
            .unwrap_or_else(|_| "/opt/deepagents/bin/dcode".into());
        let initialized = command("/usr/bin/git", config, root)
            .args(["init", "--quiet"])
            .output()
            .await?;
        if !initialized.status.success() {
            return Err(Error::Invalid("Could not initialize the agent workspace"));
        }
        let mcp = json!({"mcpServers":{"lens":{"type":"http","url":"${LENS_TOOLS_URL}","headers":{"Authorization":"Bearer ${LENS_AGENT_TOKEN}"}}}});
        write_owned(
            &root.join("mcp.json"),
            &serde_json::to_string(&mcp)?,
            config.uid,
        )
        .await?;
        let mut child = command(&executable, config, root)
            .env("DEEPAGENTS_HOME", root.join("profile"))
            .args([
                "--acp",
                "--model",
                &format!("openai:{}", config.model),
                "--mcp-config",
            ])
            .arg(root.join("mcp.json"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let input = child.stdin.take().ok_or(Error::Runtime)?;
        let output = BufReader::new(child.stdout.take().ok_or(Error::Runtime)?).lines();
        let mut process = Self {
            child,
            input,
            output,
            next_id: 0,
            loaded: HashSet::new(),
        };
        let init = process.request("initialize", json!({"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"lens","version":env!("CARGO_PKG_VERSION")}}), None).await?;
        if init.pointer("/agentCapabilities/loadSession") != Some(&Value::Bool(true)) {
            return Err(Error::Invalid(
                "Deep Agents must support persistent session loading",
            ));
        }
        Ok(process)
    }

    async fn send(&mut self, value: &Value) -> Result<(), Error> {
        self.input
            .write_all(format!("{}\n", serde_json::to_string(value)?).as_bytes())
            .await?;
        self.input.flush().await?;
        Ok(())
    }

    async fn request(
        &mut self,
        method: &str,
        params: Value,
        events: Option<&mpsc::Sender<Result<String, std::io::Error>>>,
    ) -> Result<Value, Error> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        while let Some(line) = self.output.next_line().await? {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value.get("method") == Some(&json!("session/request_permission")) {
                let option = value
                    .pointer("/params/options")
                    .and_then(Value::as_array)
                    .and_then(|options| {
                        options.iter().find(|option| option["kind"] == "allow_once")
                    })
                    .and_then(|option| option.get("optionId"));
                let outcome = option.map_or_else(
                    || json!({"outcome":"cancelled"}),
                    |id| json!({"outcome":"selected","optionId":id}),
                );
                self.send(&json!({"jsonrpc":"2.0","id":value["id"],"result":{"outcome":outcome}}))
                    .await?;
                continue;
            }
            if value.get("method") == Some(&json!("session/update")) {
                if let Some(events) = events {
                    emit(events, "update", value["params"]["update"].clone()).await?;
                }
                continue;
            }
            if value.get("method").is_some() && value.get("id").is_some() {
                self.send(&json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32601,"message":"Client method unavailable"}})).await?;
                continue;
            }
            if value.get("id") == Some(&json!(id)) {
                if value.get("error").is_some() {
                    return Err(Error::Runtime);
                }
                return value.get("result").cloned().ok_or(Error::Runtime);
            }
        }
        Err(Error::Runtime)
    }

    pub async fn prompt(
        &mut self,
        prompt: &Prompt,
        root: &Path,
        events: &mpsc::Sender<Result<String, std::io::Error>>,
    ) -> Result<(), Error> {
        let native_id = if let Some(id) = &prompt.native_id {
            if !self.loaded.contains(id) {
                self.request(
                    "session/load",
                    json!({"sessionId":id,"cwd":root.join("work"),"mcpServers":[]}),
                    None,
                )
                .await?;
                self.loaded.insert(id.clone());
            }
            id.clone()
        } else {
            let result = self
                .request(
                    "session/new",
                    json!({"cwd":root.join("work"),"mcpServers":[]}),
                    None,
                )
                .await?;
            let id = result["sessionId"]
                .as_str()
                .ok_or(Error::Runtime)?
                .to_owned();
            self.loaded.insert(id.clone());
            id
        };
        emit(events, "session", json!({"native_id":native_id})).await?;
        let result = self.request("session/prompt", json!({"sessionId":native_id,"messageId":prompt.turn_id,"prompt":[{"type":"text","text":prompt.text}]}), Some(events)).await?;
        let kind = if result["stopReason"] == "end_turn" {
            "completed"
        } else {
            "interrupted"
        };
        emit(events, kind, result).await
    }

    pub fn alive(&mut self) -> bool {
        self.child.try_wait().is_ok_and(|status| status.is_none())
    }

    pub async fn stop(&mut self) -> Result<(), Error> {
        terminate(&mut self.child).await
    }
}
