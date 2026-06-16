use async_trait::async_trait;
use uuid::Uuid;

use crate::config::{Config, StorageBackend};
#[cfg(not(feature = "sqlite"))]
use crate::error::CoworkerError;
use crate::error::Result;

pub mod json;
pub mod model;
pub mod migrate;
#[cfg(feature = "sqlite")]
pub mod sqlite;

pub use model::*;

pub use migrate::{format_migrate_summary, migrate};

use json::JsonStore;
#[cfg(feature = "sqlite")]
use sqlite::SqliteStore;

#[async_trait]
pub trait Store: Send + Sync {
    async fn save_digest(&self, digest: &Digest) -> Result<()>;
    async fn latest_digest(&self) -> Result<Option<Digest>>;
    async fn get_digest_by_skill(&self, skill: &str) -> Result<Option<Digest>>;
    async fn list_digests(&self, limit: usize) -> Result<Vec<Digest>>;

    async fn upsert_pr_snapshot(&self, snap: &PrSnapshot) -> Result<()>;
    async fn list_pr_snapshots(&self, repo: Option<&str>) -> Result<Vec<PrSnapshot>>;

    async fn push_approval(&self, item: &Approval) -> Result<()>;
    async fn get_pending_approval(&self, id: &Uuid) -> Result<Approval>;
    async fn decide_approval(&self, id: &Uuid, approve: bool) -> Result<()>;
    async fn list_pending_approvals(&self) -> Result<Vec<Approval>>;

    async fn upsert_backport_queue(&self, item: &BackportQueueItem) -> Result<()>;
    async fn list_backport_queue(&self, repo: Option<&str>) -> Result<Vec<BackportQueueItem>>;

    async fn append_audit(&self, entry: &AuditEntry) -> Result<()>;

    async fn record_flaky_incident(&self, incident: &FlakyIncident) -> Result<()>;
    async fn update_flaky_rerun(&self, incident_id: &Uuid, outcome: RerunOutcome) -> Result<()>;
    async fn list_flaky_tests(&self, q: FlakyQuery) -> Result<Vec<FlakyTestRollup>>;

    async fn record_main_alert(&self, alert: &MainAlert) -> Result<()>;
    async fn list_main_alerts(&self, q: MainAlertQuery) -> Result<Vec<MainAlert>>;
    async fn acknowledge_main_alert(&self, id: &Uuid) -> Result<()>;

    async fn start_workflow_run(&self, workflow_id: &str) -> Result<Uuid>;
    async fn finish_workflow_run(
        &self,
        id: &Uuid,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<()>;

    async fn create_chat_session(
        &self,
        title: Option<&str>,
        repo_scope: Option<&str>,
    ) -> Result<ChatSession>;
    async fn get_chat_session(&self, id: &Uuid) -> Result<Option<ChatSession>>;
    async fn update_chat_session(&self, session: &ChatSession) -> Result<()>;
    async fn append_chat_message(&self, msg: &ChatMessage) -> Result<()>;
    async fn update_chat_message(&self, msg: &ChatMessage) -> Result<()>;
    async fn list_chat_messages(&self, session_id: &Uuid, limit: usize) -> Result<Vec<ChatMessage>>;

    async fn upsert_issue_snapshot(&self, snap: &IssueSnapshot) -> Result<()>;
    async fn list_issue_snapshots(&self, repo: Option<&str>) -> Result<Vec<IssueSnapshot>>;

    async fn save_transcript(&self, t: &Transcript) -> Result<()>;
    async fn list_transcripts(&self, limit: usize) -> Result<Vec<Transcript>>;

    async fn save_regression_link(&self, link: &RegressionLink) -> Result<()>;
    async fn list_regression_links(&self, limit: usize) -> Result<Vec<RegressionLink>>;

    async fn reclassify_flaky(&self, fingerprint: &str, classification: Classification) -> Result<u32>;

    async fn list_chat_sessions(&self, limit: usize) -> Result<Vec<ChatSession>>;
}

pub fn open_store(cfg: &Config) -> Result<Box<dyn Store>> {
    match cfg.storage.backend {
        StorageBackend::Json => Ok(Box::new(JsonStore::open(cfg.storage_path())?)),
        StorageBackend::Sqlite => {
            #[cfg(feature = "sqlite")]
            {
                Ok(Box::new(SqliteStore::open(cfg.storage_path(), cfg.storage.wal)?))
            }
            #[cfg(not(feature = "sqlite"))]
            {
                Err(CoworkerError::Store(
                    "sqlite backend requires --features sqlite".into(),
                ))
            }
        }
    }
}
