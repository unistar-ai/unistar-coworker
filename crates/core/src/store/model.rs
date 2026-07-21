use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    RerunFlaky,
    Backport,
    PostComment,
    IssueAddLabel,
    WriteFile,
    EditFile,
    BashRun,
    PythonRun,
    /// Federated MCP mutating tool (`comment_body` holds JSON `{tool_name, args}`).
    McpTool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub id: Uuid,
    pub kind: ApprovalKind,
    pub repo: String,
    pub pr_number: Option<u32>,
    pub run_id: Option<i64>,
    #[serde(default)]
    pub target_branch: Option<String>,
    #[serde(default)]
    pub incident_id: Option<Uuid>,
    pub description: String,
    pub status: ApprovalStatus,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    /// Payload for PostComment approvals.
    #[serde(default)]
    pub comment_body: Option<String>,
    /// Payload for IssueAddLabel approvals.
    #[serde(default)]
    pub issue_number: Option<u32>,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackportQueueItem {
    pub id: Uuid,
    pub repo: String,
    pub pr_number: u32,
    pub pr_title: String,
    pub target_branch: String,
    pub status: BackportStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackportStatus {
    Queued,
    Approved,
    Created,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub ts: DateTime<Utc>,
    pub level: String,
    pub event: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    pub ts: DateTime<Utc>,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
    Tool,
    Harness,
    /// Materialized agent thinking summary (`[agent reasoning summary]` in LLM context).
    Reasoning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub title: String,
    #[serde(default)]
    pub repo_scope: Option<String>,
    /// Last injected runtime context revision + snapshot for delta updates across turns.
    #[serde(default)]
    pub runtime_state: ChatRuntimeState,
    /// Legacy field from the removed Pi-style message tree (always ignored).
    #[serde(default)]
    pub active_leaf_message_id: Option<Uuid>,
}

/// Persisted workspace/skills snapshot for chat runtime context deltas.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ChatRuntimeState {
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub workspace_path: String,
    #[serde(default)]
    pub git_summary: String,
    #[serde(default)]
    pub recent_edits: Vec<String>,
    #[serde(default)]
    pub loaded_skills: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: Uuid,
    pub session_id: Uuid,
    pub role: ChatRole,
    pub content: String,
    pub ts: DateTime<Utc>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_calls_json: Option<String>,
    /// Native API `tool_call_id` for `ChatRole::Tool` messages (matches
    /// assistant `tool_calls[].id`). Required when reloading history into
    /// OpenAI-compatible chat completions.
    #[serde(default)]
    pub tool_call_id: Option<String>,
    /// For `Reasoning` messages: the raw (uncompressed) thinking trace, when
    /// it differs from `content` (which holds the LLM-compressed summary).
    /// `None` for non-reasoning messages or when no compression occurred.
    #[serde(default)]
    pub reasoning_original: Option<String>,
    /// Legacy tree fields (ignored; chat history is linear by timestamp).
    #[serde(default)]
    pub parent_message_id: Option<Uuid>,
    #[serde(default)]
    pub branch_index: Option<u32>,
}
