use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use uuid::Uuid;

use crate::agent::context::harness_nudge_base;
use crate::error::{CoworkerError, Result};
use crate::store::{
    Approval, ApprovalStatus, AuditEntry, BackportQueueItem, ChatMessage, ChatRole,
    ChatRuntimeState, ChatSession, Store,
};
use async_trait::async_trait;

#[derive(Debug)]
pub struct JsonStore {
    root: PathBuf,
}

impl JsonStore {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join("approvals"))?;
        fs::create_dir_all(root.join("audit"))?;
        fs::create_dir_all(root.join("backport_queue"))?;
        fs::create_dir_all(root.join("chat/sessions"))?;
        fs::create_dir_all(root.join("chat/messages"))?;

        Ok(Self { root })
    }

    fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
        let tmp = path.with_extension("tmp");
        let data = serde_json::to_vec_pretty(value)?;
        fs::write(&tmp, data)?;
        fs::rename(tmp, path).map_err(CoworkerError::Io)?;
        Ok(())
    }

    fn append_jsonl<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
        let mut line = serde_json::to_string(value)?;
        line.push('\n');
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(line.as_bytes())?;
        Ok(())
    }

    /// Replace the trailing harness row when the base nudge is unchanged (retry counter only).
    fn append_or_replace_harness_jsonl(path: &Path, msg: &ChatMessage) -> Result<()> {
        let mut lines: Vec<String> = if path.exists() {
            fs::read_to_string(path)?
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(str::to_string)
                .collect()
        } else {
            Vec::new()
        };
        let new_base = harness_nudge_base(&msg.content);
        if let Some(last) = lines.last() {
            if let Ok(prev) = serde_json::from_str::<ChatMessage>(last) {
                if prev.role == ChatRole::Harness && harness_nudge_base(&prev.content) == new_base {
                    lines.pop();
                }
            }
        }
        let mut out = String::new();
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        out.push_str(&line);
        fs::write(path, out).map_err(CoworkerError::Io)?;
        Ok(())
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let data = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}

#[async_trait]
impl Store for JsonStore {
    async fn push_approval(&self, item: &Approval) -> Result<()> {
        let path = self.root.join("approvals/pending.json");
        let mut pending: Vec<Approval> = if path.exists() {
            read_json(&path)?
        } else {
            vec![]
        };
        pending.push(item.clone());
        Self::write_json(&path, &pending)
    }

    async fn get_pending_approval(&self, id: &Uuid) -> Result<Approval> {
        let pending = self.list_pending_approvals().await?;
        pending
            .into_iter()
            .find(|a| &a.id == id)
            .ok_or_else(|| CoworkerError::Store(format!("approval {id} not found")))
    }

    async fn decide_approval(&self, id: &Uuid, approve: bool) -> Result<()> {
        let pending_path = self.root.join("approvals/pending.json");
        let mut pending: Vec<Approval> = if pending_path.exists() {
            read_json(&pending_path)?
        } else {
            return Err(CoworkerError::Store("no pending approvals".into()));
        };
        let idx = pending
            .iter()
            .position(|a| &a.id == id)
            .ok_or_else(|| CoworkerError::Store(format!("approval {id} not found")))?;
        let mut item = pending.remove(idx);
        item.status = if approve {
            ApprovalStatus::Approved
        } else {
            ApprovalStatus::Denied
        };
        item.decided_at = Some(Utc::now());
        Self::write_json(&pending_path, &pending)?;
        Self::append_jsonl(&self.root.join("approvals/history.jsonl"), &item)
    }

    async fn list_pending_approvals(&self) -> Result<Vec<Approval>> {
        let path = self.root.join("approvals/pending.json");
        if !path.exists() {
            return Ok(vec![]);
        }
        Ok(read_json(&path)?)
    }

    async fn list_approval_history(&self, limit: usize) -> Result<Vec<Approval>> {
        let path = self.root.join("approvals/history.jsonl");
        if !path.exists() {
            return Ok(vec![]);
        }
        let raw = fs::read_to_string(&path)?;
        let mut list: Vec<Approval> = raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        list.sort_by_key(|a| std::cmp::Reverse(a.decided_at.unwrap_or(a.created_at)));
        list.truncate(limit);
        Ok(list)
    }

    async fn append_audit(&self, entry: &AuditEntry) -> Result<()> {
        let month = entry.ts.format("%Y-%m").to_string();
        let path = self.root.join(format!("audit/{month}.jsonl"));
        Self::append_jsonl(&path, entry)
    }

    async fn upsert_backport_queue(&self, item: &BackportQueueItem) -> Result<()> {
        let path = self.root.join("backport_queue/items.json");
        let mut items: HashMap<String, BackportQueueItem> = if path.exists() {
            read_json(&path)?
        } else {
            HashMap::new()
        };
        items.insert(item.id.to_string(), item.clone());
        Self::write_json(&path, &items)
    }

    async fn list_backport_queue(&self, repo: Option<&str>) -> Result<Vec<BackportQueueItem>> {
        let path = self.root.join("backport_queue/items.json");
        if !path.exists() {
            return Ok(vec![]);
        }
        let items: HashMap<String, BackportQueueItem> = read_json(&path)?;
        let mut list: Vec<_> = items
            .into_values()
            .filter(|i| repo.is_none_or(|r| i.repo == r))
            .collect();
        list.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        Ok(list)
    }

    async fn create_chat_session(
        &self,
        title: Option<&str>,
        repo_scope: Option<&str>,
    ) -> Result<ChatSession> {
        let session = ChatSession {
            id: Uuid::new_v4(),
            created_at: Utc::now(),
            title: title.unwrap_or("Chat").to_string(),
            repo_scope: repo_scope.map(str::to_string),
            runtime_state: ChatRuntimeState::default(),
            active_leaf_message_id: None,
        };
        let path = self
            .root
            .join("chat/sessions")
            .join(format!("{}.json", session.id));
        Self::write_json(&path, &session)?;
        Ok(session)
    }

    async fn get_chat_session(&self, id: &Uuid) -> Result<Option<ChatSession>> {
        let path = self.root.join("chat/sessions").join(format!("{id}.json"));
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(read_json(&path)?))
    }

    async fn update_chat_session(&self, session: &ChatSession) -> Result<()> {
        let path = self
            .root
            .join("chat/sessions")
            .join(format!("{}.json", session.id));
        Self::write_json(&path, session)
    }

    async fn delete_chat_session(&self, id: &Uuid) -> Result<()> {
        let path = self.root.join("chat/sessions").join(format!("{id}.json"));
        if !path.exists() {
            return Err(CoworkerError::Store(format!("chat session {id} not found")));
        }
        fs::remove_file(&path)?;
        let msg_path = self.root.join("chat/messages").join(format!("{id}.jsonl"));
        if msg_path.exists() {
            fs::remove_file(&msg_path)?;
        }
        Ok(())
    }

    async fn append_chat_message(&self, msg: &ChatMessage) -> Result<()> {
        if msg.role == ChatRole::Harness {
            let path = self
                .root
                .join("chat/messages")
                .join(format!("{}.jsonl", msg.session_id));
            Self::append_or_replace_harness_jsonl(&path, msg)
        } else {
            let path = self
                .root
                .join("chat/messages")
                .join(format!("{}.jsonl", msg.session_id));
            Self::append_jsonl(&path, msg)
        }
    }

    async fn update_chat_message(&self, msg: &ChatMessage) -> Result<()> {
        let path = self
            .root
            .join("chat/messages")
            .join(format!("{}.jsonl", msg.session_id));
        if !path.exists() {
            return Err(CoworkerError::Store(format!(
                "chat message file missing for session {}",
                msg.session_id
            )));
        }
        let raw = fs::read_to_string(&path)?;
        let mut found = false;
        let lines: Vec<String> = raw
            .lines()
            .map(|line| {
                if line.trim().is_empty() {
                    return line.to_string();
                }
                let Ok(prev) = serde_json::from_str::<ChatMessage>(line) else {
                    return line.to_string();
                };
                if prev.id == msg.id {
                    found = true;
                    serde_json::to_string(msg).unwrap_or_else(|_| line.to_string())
                } else {
                    line.to_string()
                }
            })
            .collect();
        if !found {
            return Err(CoworkerError::Store(format!(
                "chat message {} not found",
                msg.id
            )));
        }
        let mut out = lines.join("\n");
        if raw.ends_with('\n') {
            out.push('\n');
        }
        fs::write(&path, out)?;
        Ok(())
    }

    async fn list_chat_messages(
        &self,
        session_id: &Uuid,
        limit: usize,
    ) -> Result<Vec<ChatMessage>> {
        let path = self
            .root
            .join("chat/messages")
            .join(format!("{session_id}.jsonl"));
        if !path.exists() {
            return Ok(vec![]);
        }
        let raw = fs::read_to_string(&path)?;
        let mut msgs: Vec<ChatMessage> = raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        if msgs.len() > limit {
            msgs = msgs.split_off(msgs.len() - limit);
        }
        Ok(msgs)
    }

    async fn list_chat_sessions(&self, limit: usize) -> Result<Vec<ChatSession>> {
        let dir = self.root.join("chat/sessions");
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut sessions = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.path().extension().is_some_and(|x| x == "json") {
                if let Ok(s) = read_json::<ChatSession>(&entry.path()) {
                    sessions.push(s);
                }
            }
        }
        sessions.sort_by_key(|s| std::cmp::Reverse(s.created_at));
        sessions.truncate(limit);
        Ok(sessions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn approval_history_lists_decided_recent_first() {
        use crate::store::{ApprovalKind, ApprovalStatus};

        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::open(dir.path().to_path_buf()).unwrap();
        let older = Approval {
            id: Uuid::new_v4(),
            kind: ApprovalKind::BashRun,
            repo: "acme/widget".into(),
            pr_number: None,
            run_id: None,
            target_branch: None,
            incident_id: None,
            description: "older".into(),
            status: ApprovalStatus::Pending,
            created_at: Utc::now() - chrono::Duration::hours(2),
            decided_at: None,
            comment_body: Some(r#"{"command":"ls"}"#.into()),
            issue_number: None,
            label: None,
        };
        let newer = Approval {
            id: Uuid::new_v4(),
            kind: ApprovalKind::WriteFile,
            repo: "acme/widget".into(),
            pr_number: None,
            run_id: None,
            target_branch: None,
            incident_id: None,
            description: "newer".into(),
            status: ApprovalStatus::Pending,
            created_at: Utc::now(),
            decided_at: None,
            comment_body: Some(r#"{"path":"a.txt","content":"x"}"#.into()),
            issue_number: None,
            label: None,
        };
        store.push_approval(&older).await.unwrap();
        store.push_approval(&newer).await.unwrap();
        store.decide_approval(&older.id, false).await.unwrap();
        store.decide_approval(&newer.id, true).await.unwrap();

        let history = store.list_approval_history(10).await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].id, newer.id);
        assert_eq!(history[0].status, ApprovalStatus::Approved);
        assert_eq!(history[1].id, older.id);
        assert_eq!(history[1].status, ApprovalStatus::Denied);
    }

    #[tokio::test]
    async fn delete_chat_session_removes_session_and_messages() {
        use crate::store::{ChatMessage, ChatRole};
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::open(dir.path().to_path_buf()).unwrap();
        let session = store.create_chat_session(Some("bye"), None).await.unwrap();
        let sid = session.id;
        store
            .append_chat_message(&ChatMessage {
                id: Uuid::new_v4(),
                session_id: sid,
                role: ChatRole::User,
                content: "hello".into(),
                ts: Utc::now(),
                tool_name: None,
                tool_calls_json: None,
                tool_call_id: None,
                reasoning_original: None,
                parent_message_id: None,
                branch_index: None,
            })
            .await
            .unwrap();

        store.delete_chat_session(&sid).await.unwrap();
        assert!(store.get_chat_session(&sid).await.unwrap().is_none());
        assert!(store.list_chat_messages(&sid, 10).await.unwrap().is_empty());

        let missing = Uuid::new_v4();
        assert!(store.delete_chat_session(&missing).await.is_err());
    }
}
