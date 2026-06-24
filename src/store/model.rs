use chrono::{DateTime, NaiveDate, Utc};
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
pub struct DigestSummary {
    pub needs_attention: u32,
    pub ignorable: u32,
    pub flaky_candidates: u32,
    /// PRs with at least one LLM `policy` verdict run.
    #[serde(default)]
    pub policy_gates: u32,
    /// Wall-clock seconds for the workflow run that produced this digest.
    #[serde(default)]
    pub duration_secs: f64,
    /// False while daily-work is still publishing partial digests.
    #[serde(default = "default_digest_complete")]
    pub complete: bool,
}

fn default_digest_complete() -> bool {
    true
}

/// Human-readable duration for digest headers and CLI output.
pub fn format_duration(secs: f64) -> String {
    if secs <= 0.0 {
        return "—".into();
    }
    if secs < 60.0 {
        return format!("{secs:.1}s");
    }
    let mins = (secs / 60.0).floor() as u32;
    let rem = secs - f64::from(mins * 60);
    if rem < 0.05 {
        format!("{mins}m")
    } else {
        format!("{mins}m {rem:.0}s")
    }
}

impl DigestSummary {
    pub fn duration_label(&self) -> String {
        format_duration(self.duration_secs)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Digest {
    pub id: Uuid,
    pub date: NaiveDate,
    pub summary: DigestSummary,
    pub body_md: String,
    pub created_at: DateTime<Utc>,
    /// Workflow/agent that produced this digest (e.g. security-digest). DB column name kept for compat.
    #[serde(default)]
    pub skill: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigestMeta {
    pub id: Uuid,
    pub date: NaiveDate,
    pub summary: DigestSummary,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub skill: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrSnapshot {
    pub repo: String,
    pub number: u32,
    pub title: String,
    pub author: String,
    pub ci_summary: String,
    pub review_summary: String,
    pub is_draft: bool,
    pub fetched_at: DateTime<Utc>,
    #[serde(default)]
    pub triage_note: Option<String>,
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
pub struct WorkflowRun {
    pub id: Uuid,
    pub workflow_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    pub ts: DateTime<Utc>,
    pub level: String,
    pub message: String,
}

impl Digest {
    pub fn meta(&self) -> DigestMeta {
        DigestMeta {
            id: self.id,
            date: self.date,
            summary: self.summary.clone(),
            created_at: self.created_at,
            skill: self.skill.clone(),
        }
    }
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub id: Uuid,
    pub repo: String,
    pub pr_number: u32,
    pub workflow_id: String,
    pub turns_json: String,
    pub verdict: String,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_subminute() {
        assert_eq!(format_duration(2.84), "2.8s");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration(83.0), "1m 23s");
    }

    #[test]
    fn format_duration_zero() {
        assert_eq!(format_duration(0.0), "—");
    }
}
