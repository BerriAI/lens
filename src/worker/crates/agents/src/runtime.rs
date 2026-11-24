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
        .route("/actors/{id}/file", post(file))
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

#[derive(Deserialize)]
struct FileRequest {
    path: String,
}
async fn file(
    State(host): State<Arc<Host>>,
    headers: HeaderMap,
    RoutePath(id): RoutePath<String>,
    Json(input): Json<FileRequest>,
) -> Result<Json<Value>, Error> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    host.authorize(&headers)?;
    if uuid::Uuid::parse_str(&id).is_err() {
        return Err(Error::NotFound);
    }
    let work = host.root.join(id).join("work");
    let content = tokio::task::spawn_blocking(move || read_workspace_file(&work, &input.path))
        .await
        .map_err(|_| Error::Runtime)??;
    Ok(Json(json!({"data_base64":STANDARD.encode(content)})))
}

fn read_workspace_file(work: &Path, path: &str) -> Result<Vec<u8>, Error> {
    use nix::{
        fcntl::{OFlag, open, openat},
        sys::stat::Mode,
    };
    use std::{fs::File, io::Read, os::unix::fs::MetadataExt, path::Component};
    let parts = Path::new(path)
        .components()
        .map(|part| match part {
            Component::Normal(part) => Ok(part),
            _ => Err(Error::Invalid(
                "Use a relative path inside your working directory",
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parts.is_empty() {
        return Err(Error::Invalid("Artifact path is empty"));
    }
    let flags = OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let mut directory = File::from(
        open(work, flags | OFlag::O_DIRECTORY, Mode::empty()).map_err(|_| Error::Forbidden)?,
    );
    let owner = directory.metadata()?.uid();
    for part in &parts[..parts.len() - 1] {
        directory = File::from(
            openat(
                &directory,
                Path::new(part),
                flags | OFlag::O_DIRECTORY,
                Mode::empty(),
            )
            .map_err(|_| Error::Forbidden)?,
        );
    }
    let file = File::from(
        openat(
            &directory,
            Path::new(parts[parts.len() - 1]),
            flags | OFlag::O_NONBLOCK,
            Mode::empty(),
        )
        .map_err(|_| Error::Forbidden)?,
    );
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.uid() != owner
        || meta.len() > crate::artifacts::MAX_ARTIFACT_BYTES as u64
    {
        return Err(Error::Forbidden);
    }
    let mut bytes = Vec::new();
    file.take(crate::artifacts::MAX_ARTIFACT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > crate::artifacts::MAX_ARTIFACT_BYTES {
        return Err(Error::Invalid("Artifact exceeds 10 MiB"));
    }
    Ok(bytes)
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
    use super::read_workspace_file;
    use rstest::rstest;

    #[rstest]
    #[case::parent("../outside.txt")]
    #[case::absolute("/etc/passwd")]
    #[case::file_link("linked.txt")]
    #[case::directory_link("linked-dir/outside.txt")]
    #[case::directory("nested")]
    fn artifact_reads_cannot_escape_the_working_directory(#[case] path: &str) {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir_all(work.join("nested")).unwrap();
        std::fs::write(root.path().join("outside.txt"), b"private fixture").unwrap();
        std::os::unix::fs::symlink(root.path().join("outside.txt"), work.join("linked.txt"))
            .unwrap();
        std::os::unix::fs::symlink(root.path(), work.join("linked-dir")).unwrap();
        assert!(read_workspace_file(&work, path).is_err());
    }

    #[rstest]
    fn regular_workspace_files_can_be_published() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("charts")).unwrap();
        std::fs::write(
            root.path().join("charts/data.csv"),
            b"status,count\nfailed,3\n",
        )
        .unwrap();
        assert_eq!(
            read_workspace_file(root.path(), "charts/data.csv").unwrap(),
            b"status,count\nfailed,3\n"
        );
    }
}
