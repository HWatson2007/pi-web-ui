use std::{collections::HashMap, sync::Arc};

use anyhow::{Result, anyhow, bail};
use chrono::Utc;
use serde_json::Value;
use tokio::sync::{RwLock, broadcast};

use crate::{
    catalog,
    config::Config,
    protocol::{PromptBehavior, ServerMessage, SessionSummary},
    rpc::PiProcess,
};

pub struct SessionHub {
    config: Config,
    runtimes: RwLock<HashMap<String, Arc<PiProcess>>>,
    pub bus: broadcast::Sender<ServerMessage>,
}

impl SessionHub {
    pub fn new(config: Config) -> Self {
        let (bus, _) = broadcast::channel(1024);
        Self {
            config,
            runtimes: RwLock::new(HashMap::new()),
            bus,
        }
    }

    pub async fn list(&self) -> Vec<SessionSummary> {
        let agent_dir = self.config.agent_dir().clone();
        let mut catalog = tokio::task::spawn_blocking(move || catalog::scan(&agent_dir))
            .await
            .unwrap_or_default();
        let runtimes = self.runtimes.read().await;
        for session in &mut catalog {
            for runtime in runtimes.values() {
                let runtime_path = runtime.session_file.read().await;
                if runtime_path.as_ref() == session.path.as_ref() {
                    session.runtime_id = Some(runtime.runtime_id.clone());
                    session.running = true;
                    session.title = runtime.title.read().await.clone();
                }
            }
        }
        for runtime in runtimes.values() {
            let path = runtime.session_file.read().await.clone();
            if catalog.iter().any(|session| session.path == path) {
                continue;
            }
            catalog.push(self.runtime_summary(runtime).await);
        }
        catalog.sort_by(|left, right| right.modified_at.cmp(&left.modified_at));
        catalog
    }

    pub async fn create(&self) -> Result<Arc<PiProcess>> {
        self.ensure_capacity().await?;
        let runtime = PiProcess::spawn(&self.config, None, self.bus.clone()).await?;
        self.runtimes
            .write()
            .await
            .insert(runtime.runtime_id.clone(), Arc::clone(&runtime));
        Ok(runtime)
    }

    pub async fn open_catalog(&self, catalog_id: &str) -> Result<Arc<PiProcess>> {
        let sessions = self.list().await;
        if let Some(running) = sessions
            .iter()
            .find(|session| session.catalog_id == catalog_id)
            .and_then(|session| session.runtime_id.as_deref())
        {
            return self.get(running).await;
        }
        self.ensure_capacity().await?;
        let agent_dir = self.config.agent_dir().clone();
        let target = catalog_id.to_owned();
        let found = tokio::task::spawn_blocking(move || catalog::find_by_id(&agent_dir, &target))
            .await?
            .ok_or_else(|| anyhow!("session not found"))?;
        let path = found
            .path
            .as_deref()
            .ok_or_else(|| anyhow!("session path is unavailable"))?;
        let runtime = PiProcess::spawn(&self.config, Some(path), self.bus.clone()).await?;
        *runtime.title.write().await = found.title;
        self.runtimes
            .write()
            .await
            .insert(runtime.runtime_id.clone(), Arc::clone(&runtime));
        Ok(runtime)
    }

    pub async fn get(&self, runtime_id: &str) -> Result<Arc<PiProcess>> {
        self.runtimes
            .read()
            .await
            .get(runtime_id)
            .cloned()
            .ok_or_else(|| anyhow!("session is not running"))
    }

    pub async fn open_payload(&self, runtime: &Arc<PiProcess>) -> Result<ServerMessage> {
        let (state, messages, entries) = runtime.snapshot().await?;
        Ok(ServerMessage::SessionOpened {
            session: Box::new(self.runtime_summary(runtime).await),
            state,
            messages,
            entries,
        })
    }

    pub async fn prompt(
        &self,
        runtime_id: &str,
        message: String,
        behavior: PromptBehavior,
    ) -> Result<()> {
        let runtime = self.get(runtime_id).await?;
        if message.trim().is_empty() {
            bail!("message cannot be empty");
        }
        let is_blank = runtime.title.read().await.as_str() == "新对话";
        if is_blank {
            let title = catalog::compact_title(&message);
            if let Err(error) = runtime.set_title(title).await {
                tracing::debug!(%error, "could not set automatic session title");
            }
        }
        let behavior = match behavior {
            PromptBehavior::Normal => "normal",
            PromptBehavior::Steer => "steer",
            PromptBehavior::FollowUp => "follow_up",
        };
        runtime.prompt(&message, behavior).await
    }

    pub async fn command(&self, runtime_id: &str, kind: &str, params: Value) -> Result<()> {
        let runtime = self.get(runtime_id).await?;
        runtime.request(kind, params).await?;
        Ok(())
    }

    async fn ensure_capacity(&self) -> Result<()> {
        if self.runtimes.read().await.len() >= self.config.max_sessions {
            bail!(
                "the live session limit ({}) has been reached",
                self.config.max_sessions
            );
        }
        Ok(())
    }

    async fn runtime_summary(&self, runtime: &Arc<PiProcess>) -> SessionSummary {
        let session_file = runtime.session_file.read().await.clone();
        let catalog_id = session_file
            .as_deref()
            .and_then(|path| {
                catalog::scan(self.config.agent_dir())
                    .into_iter()
                    .find(|item| item.path.as_deref() == Some(path))
            })
            .map(|item| item.catalog_id)
            .unwrap_or_else(|| runtime.runtime_id.clone());
        SessionSummary {
            catalog_id,
            runtime_id: Some(runtime.runtime_id.clone()),
            title: runtime.title.read().await.clone(),
            cwd: runtime.cwd.display().to_string(),
            created_at: Utc::now().to_rfc3339(),
            modified_at: Utc::now().to_rfc3339(),
            message_count: 0,
            running: true,
            path: session_file,
        }
    }
}
