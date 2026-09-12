use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{Mutex, RwLock, broadcast, oneshot},
    time::timeout,
};
use tracing::warn;
use uuid::Uuid;

use crate::{config::Config, protocol::ServerMessage};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(25);

pub struct PiProcess {
    pub runtime_id: String,
    pub cwd: PathBuf,
    stdin: Mutex<ChildStdin>,
    _child: Mutex<Child>,
    pending: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    pub session_file: RwLock<Option<PathBuf>>,
    pub title: RwLock<String>,
    pub streaming: AtomicBool,
    bus: broadcast::Sender<ServerMessage>,
}

impl PiProcess {
    pub async fn spawn(
        config: &Config,
        resume_path: Option<&Path>,
        bus: broadcast::Sender<ServerMessage>,
    ) -> Result<Arc<Self>> {
        let cwd = resume_path
            .and_then(read_session_cwd)
            .unwrap_or_else(|| config.cwd().clone());
        let mut command = Command::new(&config.pi_binary);
        command
            .arg("--mode")
            .arg("rpc")
            .current_dir(&cwd)
            .env("PI_CODING_AGENT_DIR", config.agent_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(path) = resume_path {
            command.arg("--session").arg(path);
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("failed to start {}", config.pi_binary.display()))?;
        let stdin = child.stdin.take().context("Pi RPC stdin is unavailable")?;
        let stdout = child
            .stdout
            .take()
            .context("Pi RPC stdout is unavailable")?;
        let stderr = child
            .stderr
            .take()
            .context("Pi RPC stderr is unavailable")?;
        let runtime = Arc::new(Self {
            runtime_id: Uuid::new_v4().to_string(),
            cwd,
            stdin: Mutex::new(stdin),
            _child: Mutex::new(child),
            pending: Mutex::new(HashMap::new()),
            session_file: RwLock::new(resume_path.map(Path::to_path_buf)),
            title: RwLock::new("新对话".to_owned()),
            streaming: AtomicBool::new(false),
            bus,
        });

        tokio::spawn(read_stdout(Arc::clone(&runtime), stdout));
        tokio::spawn(read_stderr(runtime.runtime_id.clone(), stderr));

        let state = runtime.request("get_state", json!({})).await?;
        runtime.apply_state(&state).await;
        Ok(runtime)
    }

    pub async fn request(&self, kind: &str, params: Value) -> Result<Value> {
        let id = Uuid::new_v4().to_string();
        let mut request = match params {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        request.insert("id".to_owned(), Value::String(id.clone()));
        request.insert("type".to_owned(), Value::String(kind.to_owned()));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);

        if let Err(error) = self.write(Value::Object(request)).await {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }

        let response = timeout(REQUEST_TIMEOUT, rx)
            .await
            .map_err(|_| anyhow!("Pi RPC request timed out: {kind}"))?
            .map_err(|_| anyhow!("Pi RPC process closed while waiting for {kind}"))?;
        if response.get("success").and_then(Value::as_bool) != Some(true) {
            let message = response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Pi RPC request failed");
            bail!("{message}");
        }
        Ok(response.get("data").cloned().unwrap_or(Value::Null))
    }

    pub async fn prompt(&self, message: &str, behavior: &str) -> Result<()> {
        let mut params = json!({ "message": message });
        if behavior != "normal" || self.streaming.load(Ordering::Relaxed) {
            params["streamingBehavior"] = Value::String(if behavior == "follow_up" {
                "followUp".to_owned()
            } else {
                "steer".to_owned()
            });
        }
        self.request("prompt", params).await?;
        Ok(())
    }

    pub async fn abort(&self) -> Result<()> {
        self.request("abort", json!({})).await?;
        Ok(())
    }

    pub async fn snapshot(&self) -> Result<(Value, Vec<Value>, Vec<Value>)> {
        let state = self.request("get_state", json!({})).await?;
        self.apply_state(&state).await;
        let messages = self
            .request("get_messages", json!({}))
            .await?
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let entries = match self.request("get_entries", json!({})).await {
            Ok(data) => data
                .get("entries")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            Err(error) => {
                // Older Pi releases did not expose get_entries. Keep the
                // conversation usable and derive the timeline from messages.
                tracing::debug!(%error, "get_entries unavailable; using message timeline");
                messages
                    .iter()
                    .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
                    .map(|message| json!({ "type": "message", "message": message }))
                    .collect()
            }
        };
        Ok((state, messages, entries))
    }

    pub async fn set_title(&self, title: String) -> Result<()> {
        self.request("set_session_name", json!({ "name": title }))
            .await?;
        *self.title.write().await = title;
        Ok(())
    }

    async fn write(&self, value: Value) -> Result<()> {
        let mut payload = serde_json::to_vec(&value)?;
        payload.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&payload)
            .await
            .context("failed to write Pi RPC command")?;
        stdin
            .flush()
            .await
            .context("failed to flush Pi RPC command")?;
        Ok(())
    }

    async fn apply_state(&self, state: &Value) {
        if let Some(path) = state.get("sessionFile").and_then(Value::as_str) {
            *self.session_file.write().await = Some(PathBuf::from(path));
        }
        if let Some(name) = state.get("sessionName").and_then(Value::as_str) {
            *self.title.write().await = name.to_owned();
        }
        if let Some(streaming) = state.get("isStreaming").and_then(Value::as_bool) {
            self.streaming.store(streaming, Ordering::Relaxed);
        }
    }
}

async fn read_stdout(process: Arc<PiProcess>, stdout: tokio::process::ChildStdout) {
    let mut reader = BufReader::new(stdout);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer).await {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                warn!(runtime_id = %process.runtime_id, %error, "failed reading Pi RPC output");
                break;
            }
        }
        if buffer.last() == Some(&b'\n') {
            buffer.pop();
        }
        if buffer.last() == Some(&b'\r') {
            buffer.pop();
        }
        let value: Value = match serde_json::from_slice(&buffer) {
            Ok(value) => value,
            Err(error) => {
                warn!(runtime_id = %process.runtime_id, %error, "ignored malformed Pi RPC record");
                continue;
            }
        };

        if value.get("type").and_then(Value::as_str) == Some("response")
            && let Some(id) = value.get("id").and_then(Value::as_str)
            && let Some(waiter) = process.pending.lock().await.remove(id)
        {
            let _ = waiter.send(value);
            continue;
        }

        match value.get("type").and_then(Value::as_str) {
            Some("agent_start") => process.streaming.store(true, Ordering::Relaxed),
            Some("agent_settled") => process.streaming.store(false, Ordering::Relaxed),
            _ => {}
        }
        let _ = process.bus.send(ServerMessage::RpcEvent {
            runtime_id: process.runtime_id.clone(),
            event: value,
        });
    }
    process.streaming.store(false, Ordering::Relaxed);
    let _ = process.bus.send(ServerMessage::Error {
        message: format!("Session {} stopped", process.runtime_id),
    });
}

async fn read_stderr(runtime_id: String, stderr: tokio::process::ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        warn!(%runtime_id, pi = %line, "Pi RPC stderr");
    }
}

fn read_session_cwd(path: &Path) -> Option<PathBuf> {
    let first = std::fs::read_to_string(path)
        .ok()?
        .lines()
        .next()?
        .to_owned();
    let value: Value = serde_json::from_str(&first).ok()?;
    value.get("cwd").and_then(Value::as_str).map(PathBuf::from)
}
