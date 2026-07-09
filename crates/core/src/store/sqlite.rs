//! SQLite store backend.

use std::path::PathBuf;

use async_trait::async_trait;
use chrono::Utc;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::agent::context::harness_nudge_base;
use crate::error::{CoworkerError, Result};
use crate::store::{
    Approval, ApprovalStatus, AuditEntry, BackportQueueItem, ChatMessage, ChatRole,
    ChatRuntimeState, ChatSession, Store,
};

pub struct SqliteStore {
    path: PathBuf,
}

impl SqliteStore {
    pub fn open(path: PathBuf, wal: bool) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;
        if wal {
            conn.pragma_update(None, "journal_mode", "WAL")?;
        }
        migrate(&conn)?;
        Ok(Self { path })
    }

    pub(crate) fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = Connection::open(&self.path)?;
        f(&conn)
    }
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS approvals (
            id TEXT PRIMARY KEY,
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS audit_log (
            id TEXT PRIMARY KEY,
            ts TEXT NOT NULL,
            level TEXT NOT NULL,
            event TEXT NOT NULL,
            message TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS backport_queue (
            id TEXT PRIMARY KEY,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS chat_sessions (
            id TEXT PRIMARY KEY,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS chat_messages (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            ts TEXT NOT NULL,
            payload_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_chat_messages_session ON chat_messages(session_id, ts);
        ",
    )?;
    Ok(())
}

#[async_trait]
impl Store for SqliteStore {
    async fn push_approval(&self, item: &Approval) -> Result<()> {
        let item = item.clone();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO approvals (id, payload_json, status) VALUES (?1,?2,?3)",
                params![
                    item.id.to_string(),
                    serde_json::to_string(&item)?,
                    "pending",
                ],
            )?;
            Ok(())
        })
    }

    async fn get_pending_approval(&self, id: &Uuid) -> Result<Approval> {
        let id = *id;
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT payload_json FROM approvals WHERE id = ?1 AND status = 'pending'",
            )?;
            let mut rows = stmt.query([id.to_string()])?;
            if let Some(row) = rows.next()? {
                Ok(serde_json::from_str(&row.get::<_, String>(0)?)?)
            } else {
                Err(CoworkerError::Store(format!("approval {id} not found")))
            }
        })
    }

    async fn decide_approval(&self, id: &Uuid, approve: bool) -> Result<()> {
        let id = *id;
        self.with_conn(move |conn| {
            let status = if approve { "approved" } else { "denied" };
            conn.execute(
                "UPDATE approvals SET status = ?1 WHERE id = ?2",
                params![status, id.to_string()],
            )?;
            Ok(())
        })
    }

    async fn list_pending_approvals(&self) -> Result<Vec<Approval>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT payload_json FROM approvals WHERE status = 'pending'")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.map(|r| Ok(serde_json::from_str(&r?)?))
                .collect::<Result<Vec<_>>>()
        })
    }

    async fn list_approval_history(&self, limit: usize) -> Result<Vec<Approval>> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT payload_json, status FROM approvals WHERE status != 'pending'")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut list = Vec::new();
            for row in rows {
                let (payload, status_str) = row?;
                let mut item: Approval = serde_json::from_str(&payload)?;
                item.status = match status_str.as_str() {
                    "approved" => ApprovalStatus::Approved,
                    "denied" => ApprovalStatus::Denied,
                    _ => continue,
                };
                list.push(item);
            }
            list.sort_by_key(|a| std::cmp::Reverse(a.decided_at.unwrap_or(a.created_at)));
            list.truncate(limit);
            Ok(list)
        })
    }

    async fn append_audit(&self, entry: &AuditEntry) -> Result<()> {
        let entry = entry.clone();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO audit_log (id, ts, level, event, message) VALUES (?1,?2,?3,?4,?5)",
                params![
                    entry.id.to_string(),
                    entry.ts.to_rfc3339(),
                    entry.level,
                    entry.event,
                    entry.message,
                ],
            )?;
            Ok(())
        })
    }

    async fn upsert_backport_queue(&self, item: &BackportQueueItem) -> Result<()> {
        let item = item.clone();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO backport_queue (id, payload_json) VALUES (?1,?2)",
                params![item.id.to_string(), serde_json::to_string(&item)?],
            )?;
            Ok(())
        })
    }

    async fn list_backport_queue(&self, repo: Option<&str>) -> Result<Vec<BackportQueueItem>> {
        let repo = repo.map(str::to_string);
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare("SELECT payload_json FROM backport_queue")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut list: Vec<BackportQueueItem> = rows
                .filter_map(|r| r.ok())
                .filter_map(|j| serde_json::from_str(&j).ok())
                .filter(|i: &BackportQueueItem| repo.as_ref().is_none_or(|r| &i.repo == r))
                .collect();
            list.sort_by_key(|b| std::cmp::Reverse(b.created_at));
            Ok(list)
        })
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
        let payload = serde_json::to_string(&session)?;
        let id = session.id;
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO chat_sessions (id, payload_json) VALUES (?1,?2)",
                params![id.to_string(), payload],
            )?;
            Ok(session)
        })
    }

    async fn get_chat_session(&self, id: &Uuid) -> Result<Option<ChatSession>> {
        let sid = id.to_string();
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare("SELECT payload_json FROM chat_sessions WHERE id = ?1")?;
            let mut rows = stmt.query([&sid])?;
            if let Some(row) = rows.next()? {
                let json: String = row.get(0)?;
                return Ok(Some(serde_json::from_str(&json)?));
            }
            Ok(None)
        })
    }

    async fn update_chat_session(&self, session: &ChatSession) -> Result<()> {
        let payload = serde_json::to_string(session)?;
        let id = session.id.to_string();
        self.with_conn(move |conn| {
            conn.execute(
                "UPDATE chat_sessions SET payload_json = ?1 WHERE id = ?2",
                params![payload, id],
            )?;
            Ok(())
        })
    }

    async fn delete_chat_session(&self, id: &Uuid) -> Result<()> {
        let sid = id.to_string();
        self.with_conn(move |conn| {
            conn.execute(
                "DELETE FROM chat_messages WHERE session_id = ?1",
                params![&sid],
            )?;
            let sessions =
                conn.execute("DELETE FROM chat_sessions WHERE id = ?1", params![&sid])?;
            if sessions == 0 {
                return Err(CoworkerError::Store(format!("chat session {id} not found")));
            }
            Ok(())
        })
    }

    async fn append_chat_message(&self, msg: &ChatMessage) -> Result<()> {
        if msg.role == ChatRole::Harness {
            let sid = msg.session_id.to_string();
            let payload = serde_json::to_string(msg)?;
            let new_base = harness_nudge_base(&msg.content).to_string();
            return self.with_conn(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, payload_json FROM chat_messages WHERE session_id = ?1 ORDER BY ts DESC LIMIT 1",
                )?;
                let mut rows = stmt.query([&sid])?;
                if let Some(row) = rows.next()? {
                    let id: String = row.get(0)?;
                    let prev_json: String = row.get(1)?;
                    if let Ok(prev) = serde_json::from_str::<ChatMessage>(&prev_json) {
                        if prev.role == ChatRole::Harness
                            && harness_nudge_base(&prev.content) == new_base
                        {
                            conn.execute(
                                "UPDATE chat_messages SET payload_json = ?1, ts = ?2 WHERE id = ?3",
                                params![payload, msg.ts.to_rfc3339(), id],
                            )?;
                            return Ok(());
                        }
                    }
                }
                conn.execute(
                    "INSERT INTO chat_messages (id, session_id, ts, payload_json) VALUES (?1,?2,?3,?4)",
                    params![
                        msg.id.to_string(),
                        sid,
                        msg.ts.to_rfc3339(),
                        payload
                    ],
                )?;
                Ok(())
            });
        }

        let payload = serde_json::to_string(msg)?;
        let id = msg.id;
        let session_id = msg.session_id;
        let ts = msg.ts.to_rfc3339();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO chat_messages (id, session_id, ts, payload_json) VALUES (?1,?2,?3,?4)",
                params![id.to_string(), session_id.to_string(), ts, payload],
            )?;
            Ok(())
        })
    }

    async fn update_chat_message(&self, msg: &ChatMessage) -> Result<()> {
        let payload = serde_json::to_string(msg)?;
        let id = msg.id.to_string();
        let ts = msg.ts.to_rfc3339();
        self.with_conn(move |conn| {
            let updated = conn.execute(
                "UPDATE chat_messages SET payload_json = ?1, ts = ?2 WHERE id = ?3",
                params![payload, ts, id],
            )?;
            if updated == 0 {
                return Err(rusqlite::Error::QueryReturnedNoRows.into());
            }
            Ok(())
        })
        .map_err(|e| CoworkerError::Store(format!("update chat message: {e}")))
    }

    async fn list_chat_messages(
        &self,
        session_id: &Uuid,
        limit: usize,
    ) -> Result<Vec<ChatMessage>> {
        let sid = session_id.to_string();
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT payload_json FROM chat_messages WHERE session_id = ?1 ORDER BY ts ASC",
            )?;
            let rows = stmt.query_map([&sid], |row| row.get::<_, String>(0))?;
            let mut msgs = Vec::new();
            for row in rows {
                msgs.push(serde_json::from_str(&row?)?);
            }
            if msgs.len() > limit {
                msgs = msgs.split_off(msgs.len() - limit);
            }
            Ok(msgs)
        })
    }

    async fn list_chat_sessions(&self, limit: usize) -> Result<Vec<ChatSession>> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT payload_json FROM chat_sessions ORDER BY rowid DESC LIMIT ?1")?;
            let rows = stmt.query_map([limit as i64], |row| row.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(serde_json::from_str(&row?)?);
            }
            Ok(out)
        })
    }
}
