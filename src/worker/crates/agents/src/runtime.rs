mod acp;
mod hermes;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path as RoutePath, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::{
    process::{Child, Command},
    sync::{Mutex, mpsc, watch},
};
use tokio_stream::wrappers::ReceiverStream;

use crate::{Error, conversations::Runtime};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub actor_id: String,
    pub uid: u32,
    pub runtime: Runtime,
    pub instructions: String,
    pub model: String,
    pub lens_base: String,
    pub token: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prompt {
    pub config: Config,
    pub conversation_id: String,
    pub native_id: Option<String>,
    pub permanent: bool,
    pub turn_id: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeEvent {
    pub kind: String,
    pub data: Value,
}

pub(super) async fn emit(
    events: &mpsc::Sender<Result<String, std::io::Error>>,
    kind: &str,
    data: Value,
) -> Result<(), Error> {
    let event = RuntimeEvent {
        kind: kind.into(),
        data,
    };
    events
        .send(Ok(format!("{}\n", serde_json::to_string(&event)?)))
        .await
        .map_err(|_| Error::Runtime)
}

enum Native {
    Deepagents(acp::Process),
    Hermes(hermes::Gateway),
}
impl Native {
    fn history(&self) -> Option<hermes::History> {
        match self {
            Self::Hermes(gateway) => Some(gateway.history()),
            _ => None,
        }
    }
    fn alive(&mut self) -> bool {
        match self {
            Self::Deepagents(process) => process.alive(),
            Self::Hermes(gateway) => gateway.alive(),
        }
    }
}
struct Slot {
    native: Mutex<Option<Native>>,
    stop: watch::Sender<u64>,
    history: Mutex<Option<hermes::History>>,
}

struct Host {
    secret_hash: [u8; 32],
    root: PathBuf,
    actors: Mutex<HashMap<String, Arc<Slot>>>,
}

impl Host {
    fn authorize(&self, headers: &HeaderMap) -> Result<(), Error> {
        let supplied = headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .ok_or(Error::Forbidden)?;
        if bool::from(self.secret_hash.ct_eq(&Sha256::digest(supplied))) {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }

    async fn slot(&self, id: &str) -> Arc<Slot> {
        self.actors
            .lock()
            .await
            .entry(id.into())
            .or_insert_with(|| {
                Arc::new(Slot {
                    native: Mutex::new(None),
                    stop: watch::channel(0).0,
                    history: Mutex::new(None),
                })
            })
            .clone()
    }

    async fn prepare(&self, config: &Config) -> Result<PathBuf, Error> {
        if uuid::Uuid::parse_str(&config.actor_id).is_err()
            || !(100_001..2_000_000).contains(&config.uid)
        {
            return Err(Error::Invalid("Invalid runtime identity"));
        }
        let root = self.root.join(&config.actor_id);
        tokio::fs::create_dir_all(&root).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o711)).await?;
        }
        private_directory(&root.join("work"), config.uid).await?;
        private_directory(&root.join("home"), config.uid).await?;
        private_directory(&root.join("profile"), config.uid).await?;
        write_owned(
            &root.join("work/AGENTS.md"),
            &config.instructions,
            config.uid,
        )
        .await?;
        Ok(root)
    }
}

pub async fn serve() -> Result<(), Error> {
    if !cfg!(target_os = "linux") {
        return Err(Error::Invalid(
            "Run native agents in the Linux agent container; host execution is disabled",
        ));
    }
    let secret = std::env::var("LENS_RUNTIME_SECRET")
        .map_err(|_| Error::Invalid("Set LENS_RUNTIME_SECRET"))?;
    if secret.len() < 32 {
        return Err(Error::Invalid(
            "Use at least 32 characters for LENS_RUNTIME_SECRET",
        ));
    }
    let root = PathBuf::from(
        std::env::var("LENS_RUNTIME_DIRECTORY").unwrap_or_else(|_| "/var/lib/lens-runtime".into()),
    );
    tokio::fs::create_dir_all(&root).await?;
    let host = Arc::new(Host {
        secret_hash: Sha256::digest(secret).into(),
        root,
        actors: Mutex::new(HashMap::new()),
    });
    let routes = Router::new()
        .route("/health", get(|| async { StatusCode::NO_CONTENT }))
        .route("/prompt", post(prompt))
        .route("/actors/{id}/stop", post(stop))
        .route("/actors/{id}/history", post(history))
        .route("/actors/{id}/ensure", post(ensure))
        .with_state(host);
    let address = std::env::var("LENS_RUNTIME_ADDRESS").unwrap_or_else(|_| "0.0.0.0:4320".into());
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, routes).await?;
    Ok(())
}

async fn ensure(
    State(host): State<Arc<Host>>,
    headers: HeaderMap,
    RoutePath(id): RoutePath<String>,
    Json(config): Json<Config>,
) -> Result<StatusCode, Error> {
    host.authorize(&headers)?;
    if id != config.actor_id {
        return Err(Error::Invalid("Runtime identity mismatch"));
    }
    let slot = host.slot(&id).await;
    let Ok(mut native) = slot.native.try_lock() else {
        return Ok(StatusCode::NO_CONTENT);
    };
    let root = host.prepare(&config).await?;
    if native.as_mut().is_none_or(|native| !native.alive()) {
        *native = Some(match config.runtime {
            Runtime::Deepagents => Native::Deepagents(acp::Process::start(&config, &root).await?),
            Runtime::Hermes => Native::Hermes(hermes::Gateway::start(&config, &root).await?),
        });
    }
    *slot.history.lock().await = native.as_ref().and_then(Native::history);
    Ok(StatusCode::NO_CONTENT)
}

async fn prompt(
    State(host): State<Arc<Host>>,
    headers: HeaderMap,
    Json(prompt): Json<Prompt>,
) -> Result<impl IntoResponse, Error> {
    host.authorize(&headers)?;
    let slot = host.slot(&prompt.config.actor_id).await;
    let (sender, receiver) = mpsc::channel(128);
    let mut stopping = slot.stop.subscribe();
    tokio::spawn(async move {
        let mut native = slot.native.lock().await;
        let operation = async {
            let root = host.prepare(&prompt.config).await?;
            if let Some(Native::Deepagents(process)) = native.as_mut()
                && process.model != prompt.config.model
            {
                process.stop().await?;
                *native = None;
            }
            if native.as_mut().is_none_or(|native| !native.alive()) {
                *native = Some(match prompt.config.runtime {
                    Runtime::Deepagents => {
                        Native::Deepagents(acp::Process::start(&prompt.config, &root).await?)
                    }
                    Runtime::Hermes => {
                        Native::Hermes(hermes::Gateway::start(&prompt.config, &root).await?)
                    }
                });
            }
            *slot.history.lock().await = native.as_ref().and_then(Native::history);
            match native.as_mut().ok_or(Error::Runtime)? {
                Native::Deepagents(process) => process.prompt(&prompt, &root, &sender).await,
                Native::Hermes(gateway) => gateway.prompt(&prompt, &sender).await,
            }
        };
        let result = tokio::select! {
            result = operation => result,
            _ = stopping.changed() => Err(Error::Conflict("Agent was paused")),
        };
        if let Err(error) = result {
            if let Some(native) = native.as_mut() {
                match native {
                    Native::Deepagents(p) => {
                        let _ = p.stop().await;
                    }
                    Native::Hermes(g) => {
                        let _ = g.stop().await;
                    }
                }
            }
            let _ = emit(&sender, "failed", json!({"message":error.to_string()})).await;
            *native = None;
        }
    });
    Ok((
        [("content-type", "application/x-ndjson")],
        Body::from_stream(ReceiverStream::new(receiver)),
    ))
}

#[derive(Deserialize)]
struct HistoryRequest {
    native_id: String,
}
async fn history(
    State(host): State<Arc<Host>>,
    headers: HeaderMap,
    RoutePath(id): RoutePath<String>,
    Json(input): Json<HistoryRequest>,
) -> Result<Json<Value>, Error> {
    host.authorize(&headers)?;
    let slot = host.slot(&id).await;
    let reader = slot.history.lock().await.clone().ok_or(Error::NotFound)?;
    Ok(Json(reader.read(&input.native_id).await?))
}

async fn stop(
    State(host): State<Arc<Host>>,
    headers: HeaderMap,
    RoutePath(id): RoutePath<String>,
) -> Result<StatusCode, Error> {
    host.authorize(&headers)?;
    let slot = host.slot(&id).await;
    slot.stop.send_modify(|epoch| *epoch += 1);
    let mut native = slot.native.lock().await;
    if let Some(native) = native.as_mut() {
        match native {
            Native::Deepagents(p) => p.stop().await?,
            Native::Hermes(g) => g.stop().await?,
        }
    }
    *native = None;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) fn command(executable: &str, config: &Config, root: &Path) -> Command {
    let mut cmd = Command::new(executable);
    cmd.env_clear()
        .env(
            "PATH",
            format!(
                "{}:/usr/local/bin:/usr/bin:/bin",
                Path::new(executable)
                    .parent()
                    .unwrap_or_else(|| Path::new("/usr/local/bin"))
                    .display()
            ),
        )
        .env("LENS_WORKSPACE_DIRECTORY", root.join("work"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("HOME", root.join("home"))
        .env("LANG", "C.UTF-8")
        .env("TMPDIR", root.join("work"))
        .env("OPENAI_API_KEY", &config.token)
        .env(
            "OPENAI_BASE_URL",
            format!("{}/v1", config.lens_base.trim_end_matches('/')),
        )
        .env(
            "OPENAI_API_BASE",
            format!("{}/v1", config.lens_base.trim_end_matches('/')),
        )
        .env(
            "LENS_TOOLS_URL",
            format!("{}/mcp", config.lens_base.trim_end_matches('/')),
        )
        .env("LENS_AGENT_TOKEN", &config.token)
        .env("LANGSMITH_TRACING", "false")
        .current_dir(root.join("work"))
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(target_os = "linux")]
    {
        cmd.uid(config.uid).gid(config.uid);
    }
    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

async fn private_directory(path: &Path, uid: u32) -> Result<(), Error> {
    tokio::fs::create_dir_all(path).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
    }
    own(path, uid)?;
    Ok(())
}

pub(super) async fn write_owned(path: &Path, content: &str, uid: u32) -> Result<(), Error> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(content.as_bytes())?;
    #[cfg(target_os = "linux")]
    nix::unistd::fchown(
        &file,
        Some(nix::unistd::Uid::from_raw(uid)),
        Some(nix::unistd::Gid::from_raw(uid)),
    )
    .map_err(|_| Error::Runtime)?;
    #[cfg(not(target_os = "linux"))]
    let _ = uid;
    Ok(())
}

fn own(path: &Path, uid: u32) -> Result<(), Error> {
    #[cfg(target_os = "linux")]
    std::os::unix::fs::chown(path, Some(uid), Some(uid))?;
    #[cfg(not(target_os = "linux"))]
    let _ = (path, uid);
    Ok(())
}

pub(super) async fn terminate(child: &mut Child) -> Result<(), Error> {
    #[cfg(unix)]
    if let Some(id) = child.id() {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(id as i32),
            nix::sys::signal::Signal::SIGTERM,
        );
    }
    if tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .is_err()
    {
        #[cfg(unix)]
        if let Some(id) = child.id() {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(id as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        child.kill().await?;
        child.wait().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, path_regex},
    };

    #[rstest]
    #[case::main(Runtime::Deepagents)]
    #[case::bot(Runtime::Hermes)]
    #[tokio::test]
    #[ignore = "requires the Linux native agent image and isolated loopback"]
    async fn native_model_inputs_exclude_host_execution(#[case] runtime: Runtime) {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/mcp"))
            .respond_with(|request: &wiremock::Request| {
                let body: Value = request.body_json().unwrap();
                let result = match body["method"].as_str().unwrap() {
                    "initialize" => json!({"protocolVersion":body["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}}),
                    "notifications/initialized" => return ResponseTemplate::new(202),
                    "tools/list" => json!({"tools":[{"name":"run_python","description":"Run Python in the Lens sandbox","inputSchema":{"type":"object","properties":{"code":{"type":"string"}}}}]}),
                    other => panic!("Unexpected MCP method {other}"),
                };
                ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
            }).mount(&server).await;
        Mock::given(method("GET")).and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"object":"list","data":[{"id":"fixture-model","object":"model","created":0,"owned_by":"fixture"}]})))
            .mount(&server).await;
        let marker = format!("/tmp/lens-unconfined-{}", uuid::Uuid::new_v4());
        let command = format!("touch {marker}");
        let deferred = matches!(runtime, Runtime::Hermes);
        let round = AtomicUsize::new(0);
        Mock::given(method("POST")).and(path_regex("^/v1/(chat/completions|responses)$"))
            .respond_with(move |request: &wiremock::Request| {
                let body: Value = request.body_json().unwrap();
                let step = if body["tools"].is_array() { round.fetch_add(1, Ordering::SeqCst) } else { usize::MAX };
                let call = match (deferred, step) {
                    (true, 0) => Some(("find-sandbox", "tool_search", json!({"queries":["run_python"]}))),
                    (true, 1) => Some(("blocked-execution", "tool_call", json!({"calls":[{"name":"terminal","arguments":{"command":command}}]}))),
                    (false, 0) => Some(("blocked-execution", "execute", json!({"command":command}))),
                    _ => None,
                };
                if request.url.path() == "/v1/responses" {
                    let item = if let Some((id, name, arguments)) = &call {
                        json!({"type":"function_call","id":"fc_fixture","call_id":id,"name":name,"arguments":arguments.to_string(),"status":"completed"})
                    } else {
                        json!({"type":"message","id":"msg_fixture","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Completed","annotations":[]}]})
                    };
                    let response = json!({"id":"resp_fixture","object":"response","created_at":0,"status":"completed","model":"fixture-model","output":[item],"error":null,"incomplete_details":null,"parallel_tool_calls":false,"tool_choice":"auto","tools":[],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2,"input_tokens_details":{"cached_tokens":0},"output_tokens_details":{"reasoning_tokens":0}}});
                    if body["stream"] != true { return ResponseTemplate::new(200).set_body_json(response); }
                    let events = [
                        json!({"type":"response.created","response":response}),
                        json!({"type":"response.output_item.added","output_index":0,"item":item}),
                        json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"item_id":"msg_fixture","delta":if call.is_none() {"Completed"} else {""}}),
                        json!({"type":"response.completed","response":response}),
                    ];
                    return ResponseTemplate::new(200).insert_header("content-type","text/event-stream")
                        .set_body_string(events.iter().enumerate().map(|(sequence, event)| {
                            let mut event = event.clone(); event["sequence_number"] = json!(sequence);
                            format!("event: {}\ndata: {event}\n\n",event["type"].as_str().unwrap())
                        }).collect::<String>());
                }
                let message = if let Some((id, name, arguments)) = &call {
                    json!({"role":"assistant","content":null,"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]})
                } else { json!({"role":"assistant","content":"Completed"}) };
                let reason = if call.is_some() {"tool_calls"} else {"stop"};
                let mut response = json!({"id":"fixture","created":0,"model":"fixture-model","usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}});
                if body["stream"] == true {
                    response["object"] = json!("chat.completion.chunk");
                    response["choices"] = json!([{"index":0,"delta":message,"finish_reason":reason}]);
                    ResponseTemplate::new(200).insert_header("content-type","text/event-stream")
                        .set_body_string(format!("data: {response}\n\ndata: [DONE]\n\n"))
                } else {
                    response["object"] = json!("chat.completion");
                    response["choices"] = json!([{"index":0,"message":message,"finish_reason":reason}]);
                    ResponseTemplate::new(200).set_body_json(response)
                }
            }).mount(&server).await;
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o711))
                .unwrap();
        }
        let host = Host {
            secret_hash: [0; 32],
            root: directory.path().into(),
            actors: Mutex::new(HashMap::new()),
        };
        let config = Config {
            actor_id: uuid::Uuid::new_v4().to_string(),
            uid: 100_001,
            runtime,
            instructions: "Analyze traces using Lens tools and sandboxed Python.".into(),
            model: "fixture-model".into(),
            lens_base: server.uri(),
            token: "fixture-agent-token".into(),
        };
        let root = host.prepare(&config).await.unwrap();
        let prompt = Prompt {
            config: config.clone(),
            conversation_id: uuid::Uuid::new_v4().to_string(),
            native_id: None,
            permanent: false,
            turn_id: uuid::Uuid::new_v4().to_string(),
            text: "Acknowledge the task.".into(),
        };
        let (events, mut receiver) = mpsc::channel(256);
        let drain = tokio::spawn(async move { while receiver.recv().await.is_some() {} });
        let result = match config.runtime {
            Runtime::Deepagents => {
                let mut native = acp::Process::start(&config, &root).await.unwrap();
                let result = tokio::time::timeout(
                    Duration::from_secs(45),
                    native.prompt(&prompt, &root, &events),
                )
                .await;
                native.stop().await.unwrap();
                result
            }
            Runtime::Hermes => {
                let mut native = hermes::Gateway::start(&config, &root).await.unwrap();
                let result =
                    tokio::time::timeout(Duration::from_secs(45), native.prompt(&prompt, &events))
                        .await;
                native.stop().await.unwrap();
                result
            }
        };
        drain.abort();
        let result = result.expect("Native fixture turn timed out");
        assert!(result.is_ok(), "Native fixture turn failed: {result:?}");
        assert!(
            !Path::new(&marker).exists(),
            "A model-requested host command executed"
        );
        let requests = server.received_requests().await.unwrap();
        assert!(
            requests.iter().any(|request| {
                request.url.path().starts_with("/v1/")
                    && String::from_utf8_lossy(&request.body).contains(&config.instructions)
            }),
            "Workspace instructions did not reach the native model input"
        );
        assert!(
            requests.iter().any(|request| {
                request.body_json::<Value>().ok().is_some_and(|body| {
                    body.get("messages")
                        .or_else(|| body.get("input"))
                        .and_then(Value::as_array)
                        .is_some_and(|messages| {
                            messages.iter().any(|message| {
                                message["tool_call_id"] == "blocked-execution"
                                    || (message["type"] == "function_call_output"
                                        && message["call_id"] == "blocked-execution")
                            })
                        })
                })
            }),
            "The refused tool call was not returned to the model"
        );
        let names: Vec<String> = requests
            .iter()
            .filter(|request| {
                matches!(request.url.path(), "/v1/chat/completions" | "/v1/responses")
            })
            .flat_map(|request| {
                request.body_json::<Value>().unwrap()["tools"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .filter_map(|tool| {
                tool["function"]["name"]
                    .as_str()
                    .or_else(|| tool["name"].as_str())
                    .map(str::to_owned)
            })
            .collect();
        assert!(
            requests
                .iter()
                .any(|request| request.url.path().starts_with("/v1/")
                    && String::from_utf8_lossy(&request.body).contains("run_python")),
            "Sandbox tool missing from model input: {names:?}"
        );
        let forbidden = [
            "execute",
            "terminal",
            "process_manage",
            "execute_code",
            "js_eval",
            "write_file",
            "edit_file",
            "patch",
            "delete",
            "cronjob_manage",
            "delegate_task",
            "skill_manage",
        ];
        assert!(
            !names.iter().any(|name| forbidden.contains(&name.as_str())),
            "Host execution tool reached model input: {names:?}"
        );
    }
}
