use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    LlmFlaky,
    LlmReal,
    UserFlaky,
    UserReal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RerunOutcome {
    Pending,
    Succeeded,
    Failed,
    Skipped,
}

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigestSummary {
    pub needs_attention: u32,
    pub ignorable: u32,
    pub flaky_candidates: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Digest {
    pub id: Uuid,
    pub date: NaiveDate,
    pub summary: DigestSummary,
    pub body_md: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigestMeta {
    pub id: Uuid,
    pub date: NaiveDate,
    pub summary: DigestSummary,
    pub created_at: DateTime<Utc>,
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
pub struct FlakyIncident {
    pub id: Uuid,
    pub ts: DateTime<Utc>,
    pub repo: String,
    pub pr_number: Option<u32>,
    pub run_id: i64,
    pub workflow: String,
    pub job: Option<String>,
    pub step: Option<String>,
    pub test_name: Option<String>,
    pub fingerprint: String,
    pub classification: Classification,
    pub log_excerpt: String,
    pub llm_reason: Option<String>,
    pub rerun_outcome: Option<RerunOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakyTestRollup {
    pub fingerprint: String,
    pub repo: String,
    pub workflow: String,
    pub job: Option<String>,
    pub test_name: Option<String>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub incident_count: u32,
    pub rerun_attempts: u32,
    pub rerun_successes: u32,
    pub last_error_signature: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FlakyQuery {
    pub repo: Option<String>,
    pub since_days: Option<u32>,
    pub limit: usize,
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
        }
    }
}

pub fn compute_fingerprint(
    repo: &str,
    workflow: &str,
    job: Option<&str>,
    test_name: Option<&str>,
    error_sig: &str,
) -> String {
    use sha2::{Digest as _, Sha256};
    let job = job.unwrap_or("");
    let test = test_name.unwrap_or("");
    let fallback = if test.is_empty() { error_sig } else { test };
    let payload = format!("{repo}|{workflow}|{job}|{fallback}");
    let hash = Sha256::digest(payload.as_bytes());
    format!("{:x}", hash)
}
