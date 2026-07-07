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
    async fn save_digest(&self, digest: &Digest) -> Result<()>;
    async fn latest_digest(&self) -> Result<Option<Digest>>;
    async fn list_digests(&self, limit: usize) -> Result<Vec<Digest>>;

    async fn upsert_pr_snapshot(&self, snap: &PrSnapshot) -> Result<()>;
    async fn list_pr_snapshots(&self, repo: Option<&str>) -> Result<Vec<PrSnapshot>>;

    async fn push_approval(&self, item: &Approval) -> Result<()>;
    async fn get_pending_approval(&self, id: &Uuid) -> Result<Approval>;
    async fn decide_approval(&self, id: &Uuid, approve: bool) -> Result<()>;
    async fn list_pending_approvals(&self) -> Result<Vec<Approval>>;
    async fn list_approval_history(&self, limit: usize) -> Result<Vec<Approval>>;

    async fn upsert_backport_queue(&self, item: &BackportQueueItem) -> Result<()>;
    async fn list_backport_queue(&self, repo: Option<&str>) -> Result<Vec<BackportQueueItem>>;

    async fn append_audit(&self, entry: &AuditEntry) -> Result<()>;

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
    async fn delete_chat_session(&self, id: &Uuid) -> Result<()>;
    async fn append_chat_message(&self, msg: &ChatMessage) -> Result<()>;
    async fn update_chat_message(&self, msg: &ChatMessage) -> Result<()>;
    async fn list_chat_messages(&self, session_id: &Uuid, limit: usize)
        -> Result<Vec<ChatMessage>>;

    /// Return the active branch (root → active leaf) of a session as a
    /// chronological message list. For linear sessions (no branching) this is
    /// equivalent to `list_chat_messages` over the whole tail. The active leaf
    /// is taken from `session.active_leaf_message_id`; when `None` the latest
    /// message by timestamp is used.
    async fn list_active_branch_messages(
        &self,
        session: &ChatSession,
        limit: usize,
    ) -> Result<Vec<ChatMessage>> {
        let all = self.list_chat_messages(&session.id, usize::MAX).await?;
        let has_branching = all.iter().any(|m| m.parent_message_id.is_some());
        if !has_branching {
            let take = limit.min(all.len());
            return Ok(all[all.len().saturating_sub(take)..].to_vec());
        }
        let leaf = session
            .active_leaf_message_id
            .filter(|id| all.iter().any(|m| m.id == *id))
            .or_else(|| all.last().map(|m| m.id))
            .unwrap_or_else(Uuid::nil);
        let mut path = branch_path_to_root(&all, leaf);
        if path.len() > limit {
            path = path.split_off(path.len() - limit);
        }
        Ok(path)
    }

    async fn save_transcript(&self, t: &Transcript) -> Result<()>;
    async fn list_transcripts(&self, limit: usize) -> Result<Vec<Transcript>>;

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

/// Walk from `leaf_id` up to the root following `parent_message_id`,
/// returning the path in chronological (root → leaf) order. Guards against
/// missing parents and cycles by stopping when a node is revisited or absent.
pub fn branch_path_to_root(messages: &[ChatMessage], leaf_id: Uuid) -> Vec<ChatMessage> {
    if leaf_id == Uuid::nil() {
        return Vec::new();
    }
    let by_id: std::collections::HashMap<Uuid, &ChatMessage> =
        messages.iter().map(|m| (m.id, m)).collect();
    let mut chain: Vec<ChatMessage> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cursor = Some(leaf_id);
    while let Some(id) = cursor {
        if !seen.insert(id) {
            break;
        }
        match by_id.get(&id) {
            Some(msg) => {
                chain.push((*msg).clone());
                cursor = msg.parent_message_id;
            }
            None => break,
        }
    }
    chain.reverse();
    chain
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn msg(id: Uuid, parent: Option<Uuid>) -> ChatMessage {
        ChatMessage {
            id,
            session_id: Uuid::nil(),
            role: ChatRole::Assistant,
            content: String::new(),
            ts: Utc::now(),
            tool_name: None,
            tool_calls_json: None,
            reasoning_original: None,
            parent_message_id: parent,
            branch_index: None,
        }
    }

    #[test]
    fn branch_path_walks_to_root_in_order() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let messages = vec![
            msg(a, None),    // root
            msg(b, Some(a)), // child of a
            msg(c, Some(b)), // child of b
        ];
        let path = branch_path_to_root(&messages, c);
        assert_eq!(path.len(), 3);
        assert_eq!(path[0].id, a);
        assert_eq!(path[1].id, b);
        assert_eq!(path[2].id, c);
    }

    #[test]
    fn branch_path_stops_on_cycle() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        // b's parent is a; a's parent is b → cycle
        let messages = vec![msg(a, Some(b)), msg(b, Some(a))];
        let path = branch_path_to_root(&messages, a);
        assert!(path.len() <= 2);
    }
}
