use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    ListSessions,
    CreateSession,
    OpenSession {
        catalog_id: String,
    },
    ActivateSession {
        runtime_id: String,
    },
    Prompt {
        runtime_id: String,
        message: String,
        #[serde(default)]
        behavior: PromptBehavior,
    },
    Abort {
        runtime_id: String,
    },
    Compact {
        runtime_id: String,
    },
    SetModel {
        runtime_id: String,
        provider: String,
        model_id: String,
    },
    SetThinkingLevel {
        runtime_id: String,
        level: String,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptBehavior {
    #[default]
    Normal,
    Steer,
    FollowUp,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Hello {
        protocol: u8,
    },
    Sessions {
        sessions: Vec<SessionSummary>,
    },
    SessionOpened {
        session: Box<SessionSummary>,
        state: Value,
        messages: Vec<Value>,
        entries: Vec<Value>,
    },
    RpcEvent {
        runtime_id: String,
        event: Value,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub catalog_id: String,
    pub runtime_id: Option<String>,
    pub title: String,
    pub cwd: String,
    pub created_at: String,
    pub modified_at: String,
    pub message_count: usize,
    pub running: bool,
    #[serde(skip_serializing)]
    pub path: Option<std::path::PathBuf>,
}
