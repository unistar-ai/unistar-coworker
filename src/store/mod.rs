use async_trait::async_trait;
use uuid::Uuid;

use crate::config::{Config, StorageBackend};
#[cfg(not(feature = "sqlite"))]
use crate::error::CoworkerError;
use crate::error::Result;

pub mod json;
pub mod model;
#[cfg(feature = "sqlite")]
pub mod sqlite;

pub use model::*;

use json::JsonStore;
#[cfg(feature = "sqlite")]
use sqlite::SqliteStore;

#[async_trait]
pub trait Store: Send + Sync {
    async fn save_digest(&self, digest: &Digest) -> Result<()>;
    async fn latest_digest(&self) -> Result<Option<Digest>>;
    async fn list_digests(&self, limit: usize) -> Result<Vec<DigestMeta>>;

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

    async fn start_workflow_run(&self, workflow_id: &str) -> Result<Uuid>;
    async fn finish_workflow_run(
        &self,
        id: &Uuid,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<()>;
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
