use async_trait::async_trait;
use uuid::Uuid;

use crate::config::{Config, StorageBackend};
use crate::error::Result;

pub mod compact;
pub mod json;
pub mod migrate;
pub mod model;
pub mod sqlite;

pub use model::*;

pub use compact::{compact, CompactOptions, CompactStats};
pub use migrate::{migrate, MigrateStats};

use json::JsonStore;
use sqlite::SqliteStore;

#[async_trait]
pub trait Store: Send + Sync {
    async fn push_approval(&self, item: &Approval) -> Result<()>;
    async fn get_pending_approval(&self, id: &Uuid) -> Result<Approval>;
    async fn decide_approval(
        &self,
        id: &Uuid,
        approve: bool,
        decision_reason: Option<&str>,
    ) -> Result<()>;
    async fn list_pending_approvals(&self) -> Result<Vec<Approval>>;
    async fn list_approval_history(&self, limit: usize) -> Result<Vec<Approval>>;

    async fn upsert_backport_queue(&self, item: &BackportQueueItem) -> Result<()>;
    async fn list_backport_queue(&self, repo: Option<&str>) -> Result<Vec<BackportQueueItem>>;

    async fn append_audit(&self, entry: &AuditEntry) -> Result<()>;

    async fn create_chat_session(
        &self,
        title: Option<&str>,
        repo_scope: Option<&str>,
    ) -> Result<ChatSession>;
    async fn get_chat_session(&self, id: &Uuid) -> Result<Option<ChatSession>>;
    async fn update_chat_session(&self, session: &ChatSession) -> Result<()>;
    async fn delete_chat_session(&self, id: &Uuid) -> Result<()>;
    async fn append_chat_message(&self, msg: &ChatMessage) -> Result<()>;
    async fn update_chat_message(&self, msg: &ChatMessage) -> Result<()>;
    async fn list_chat_messages(&self, session_id: &Uuid, limit: usize)
        -> Result<Vec<ChatMessage>>;

    /// Remove `from_message_id` and all later messages in the session (linear transcript).
    async fn truncate_chat_messages_from(
        &self,
        session_id: &Uuid,
        from_message_id: Uuid,
    ) -> Result<()>;

    async fn list_chat_sessions(&self, limit: usize) -> Result<Vec<ChatSession>>;
}

pub fn open_store(cfg: &Config) -> Result<Box<dyn Store>> {
    match cfg.storage.backend {
        StorageBackend::Json => Ok(Box::new(JsonStore::open(cfg.storage_path())?)),
        StorageBackend::Sqlite => Ok(Box::new(SqliteStore::open(
            cfg.storage_path(),
            cfg.storage.wal,
        )?)),
    }
}
