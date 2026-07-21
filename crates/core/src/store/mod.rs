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
    async fn decide_approval(&self, id: &Uuid, approve: bool) -> Result<()>;
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
        if all.is_empty() {
            return Ok(vec![]);
        }
        if !session_tree_has_branching(&all) || session_tree_is_fragmented(&all) {
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

/// True when any message records a branch parent (Pi-style tree).
pub fn session_tree_has_branching(messages: &[ChatMessage]) -> bool {
    messages.iter().any(|m| m.parent_message_id.is_some())
}

/// True when the session tree has multiple disconnected roots — usually from
/// messages persisted without `parent_message_id` before chaining was enforced.
/// In that case UI/export should fall back to the linear chronological tail.
pub fn session_tree_is_fragmented(messages: &[ChatMessage]) -> bool {
    if messages.len() < 2 {
        return false;
    }
    messages
        .iter()
        .filter(|m| m.parent_message_id.is_none())
        .count()
        > 1
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

/// Final assistant replies are parented directly to the user (for regenerate
/// siblings), while tool/reasoning/harness rows form a separate process chain
/// under the same user. Walking only the active leaf therefore drops process
/// history. Expand each user→answer spine edge with those process siblings.
pub fn expand_branch_with_process_messages(
    all: &[ChatMessage],
    branch: &[ChatMessage],
) -> Vec<ChatMessage> {
    if branch.is_empty() || all.is_empty() {
        return branch.to_vec();
    }

    // Always merge process siblings under each user. Do NOT early-return when the
    // spine already contains Tool/Reasoning rows: after `ask_user`, the answer
    // user message is parented to the ask_user tool, so the active-leaf walk
    // includes that tool on the path — but bash_run / reasoning siblings under
    // the user are still missing and must be expanded.

    let mut include: std::collections::HashSet<Uuid> = branch.iter().map(|m| m.id).collect();
    let selected_assistants: std::collections::HashSet<Uuid> = branch
        .iter()
        .filter(|m| m.role == ChatRole::Assistant)
        .map(|m| m.id)
        .collect();

    for user in branch.iter().filter(|m| m.role == ChatRole::User) {
        for child in all.iter().filter(|m| m.parent_message_id == Some(user.id)) {
            if is_inactive_assistant_sibling(child, &selected_assistants) {
                continue;
            }
            insert_subtree(all, child.id, &mut include);
        }
    }

    all.iter()
        .filter(|m| include.contains(&m.id))
        .cloned()
        .collect()
}

fn is_inactive_assistant_sibling(
    child: &ChatMessage,
    selected_assistants: &std::collections::HashSet<Uuid>,
) -> bool {
    if child.role != ChatRole::Assistant {
        return false;
    }
    if selected_assistants.contains(&child.id) {
        return false;
    }
    // Final-answer regenerations carry `branch_index`. Native tool-call
    // carrier assistants parent under the user with `branch_index: None`.
    if child.branch_index.is_some() {
        return true;
    }
    // Legacy rows without branch_index: keep carriers that recorded tool calls.
    child
        .tool_calls_json
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
}

fn insert_subtree(all: &[ChatMessage], root: Uuid, include: &mut std::collections::HashSet<Uuid>) {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !include.insert(id) {
            continue;
        }
        for child in all.iter().filter(|m| m.parent_message_id == Some(id)) {
            stack.push(child.id);
        }
    }
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
            tool_call_id: None,
            reasoning_original: None,
            parent_message_id: parent,
            branch_index: None,
        }
    }

    fn msg_role(
        id: Uuid,
        parent: Option<Uuid>,
        role: ChatRole,
        branch_index: Option<u32>,
    ) -> ChatMessage {
        ChatMessage {
            id,
            session_id: Uuid::nil(),
            role,
            content: String::new(),
            ts: Utc::now(),
            tool_name: None,
            tool_calls_json: None,
            tool_call_id: None,
            reasoning_original: None,
            parent_message_id: parent,
            branch_index,
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

    #[test]
    fn fragmented_tree_detects_multiple_roots() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let messages = vec![msg(a, None), msg(b, None)];
        assert!(session_tree_is_fragmented(&messages));
    }

    #[test]
    fn coherent_branch_tree_is_not_fragmented() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let messages = vec![msg(a, None), msg(b, Some(a)), msg(c, Some(b))];
        assert!(!session_tree_is_fragmented(&messages));
    }

    #[test]
    fn expand_branch_includes_process_siblings_under_user() {
        let user = Uuid::new_v4();
        let reasoning = Uuid::new_v4();
        let tool = Uuid::new_v4();
        let interim = Uuid::new_v4();
        let answer = Uuid::new_v4();
        let stale = Uuid::new_v4();
        let all = vec![
            msg_role(user, None, ChatRole::User, None),
            msg_role(reasoning, Some(user), ChatRole::Reasoning, None),
            msg_role(tool, Some(reasoning), ChatRole::Tool, None),
            msg_role(interim, Some(tool), ChatRole::Assistant, None),
            msg_role(stale, Some(user), ChatRole::Assistant, Some(0)),
            msg_role(answer, Some(user), ChatRole::Assistant, Some(1)),
        ];
        let branch = vec![
            all[0].clone(), // user
            all[5].clone(), // active answer
        ];
        let expanded = expand_branch_with_process_messages(&all, &branch);
        let ids: Vec<_> = expanded.iter().map(|m| m.id).collect();
        assert!(ids.contains(&user));
        assert!(ids.contains(&reasoning));
        assert!(ids.contains(&tool));
        assert!(ids.contains(&interim));
        assert!(ids.contains(&answer));
        assert!(
            !ids.contains(&stale),
            "inactive regenerate sibling must stay excluded"
        );
        assert_eq!(expanded.len(), 5);
    }

    #[test]
    fn expand_branch_keeps_tool_call_carrier_assistants() {
        let user = Uuid::new_v4();
        let carrier = Uuid::new_v4();
        let tool = Uuid::new_v4();
        let answer = Uuid::new_v4();
        let mut carrier_msg = msg_role(carrier, Some(user), ChatRole::Assistant, None);
        carrier_msg.tool_calls_json = Some(r#"[{"name":"bash_run"}]"#.into());
        let all = vec![
            msg_role(user, None, ChatRole::User, None),
            carrier_msg,
            msg_role(tool, Some(carrier), ChatRole::Tool, None),
            msg_role(answer, Some(user), ChatRole::Assistant, Some(0)),
        ];
        let branch = vec![all[0].clone(), all[3].clone()];
        let expanded = expand_branch_with_process_messages(&all, &branch);
        let ids: Vec<_> = expanded.iter().map(|m| m.id).collect();
        assert!(ids.contains(&carrier));
        assert!(ids.contains(&tool));
        assert!(ids.contains(&answer));
    }

    /// ask_user answer users parent to the ask_user tool, so the active-leaf
    /// spine already contains a Tool row — expansion must still pull in the
    /// process siblings under that user (regression: UI showed only the final
    /// answer with zero tools/reasoning).
    #[test]
    fn expand_branch_after_ask_user_spine_still_includes_process() {
        let user1 = Uuid::new_v4();
        let ask_carrier = Uuid::new_v4();
        let ask_tool = Uuid::new_v4();
        let user_answer = Uuid::new_v4();
        let reasoning = Uuid::new_v4();
        let bash = Uuid::new_v4();
        let final_answer = Uuid::new_v4();

        let mut ask_carrier_msg = msg_role(ask_carrier, Some(user1), ChatRole::Assistant, None);
        ask_carrier_msg.tool_calls_json = Some(r#"[{"name":"ask_user"}]"#.into());
        let mut ask_tool_msg = msg_role(ask_tool, Some(ask_carrier), ChatRole::Tool, None);
        ask_tool_msg.tool_name = Some("ask_user".into());
        let mut bash_msg = msg_role(bash, Some(reasoning), ChatRole::Tool, None);
        bash_msg.tool_name = Some("bash_run".into());

        let all = vec![
            msg_role(user1, None, ChatRole::User, None),
            ask_carrier_msg,
            ask_tool_msg,
            // Answer user is parented to the ask_user tool (resume path).
            msg_role(user_answer, Some(ask_tool), ChatRole::User, Some(0)),
            msg_role(reasoning, Some(user_answer), ChatRole::Reasoning, None),
            bash_msg,
            msg_role(
                final_answer,
                Some(user_answer),
                ChatRole::Assistant,
                Some(0),
            ),
        ];
        // Active-leaf walk: final → user_answer → ask_tool → ask_carrier → user1
        let branch = branch_path_to_root(&all, final_answer);
        assert!(
            branch.iter().any(|m| m.role == ChatRole::Tool),
            "precondition: ask_user tool is on the spine"
        );
        assert!(
            !branch.iter().any(|m| m.id == bash),
            "precondition: bash_run is not on the spine"
        );

        let expanded = expand_branch_with_process_messages(&all, &branch);
        let ids: Vec<_> = expanded.iter().map(|m| m.id).collect();
        assert!(ids.contains(&reasoning), "reasoning under answer user");
        assert!(ids.contains(&bash), "bash_run under answer user");
        assert!(ids.contains(&final_answer));
        assert!(ids.contains(&ask_tool));
    }
}
