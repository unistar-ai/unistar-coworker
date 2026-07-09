use std::fs;
use std::path::{Path, PathBuf};

use crate::config::StorageBackend;
use crate::error::{CoworkerError, Result};

use super::json::JsonStore;
use super::sqlite::SqliteStore;

#[derive(Debug, Default, Clone)]
pub struct MigrateStats {
    pub approvals: u32,
    pub backport_items: u32,
    pub chat_messages: u32,
}

pub async fn migrate(
    from: StorageBackend,
    to: StorageBackend,
    source_path: PathBuf,
    dest_path: PathBuf,
    wal: bool,
) -> Result<MigrateStats> {
    match (from, to) {
        (StorageBackend::Json, StorageBackend::Sqlite) => {
            migrate_json_to_sqlite(&source_path, &dest_path, wal).await
        }
        (StorageBackend::Sqlite, StorageBackend::Json) => {
            migrate_sqlite_to_json(&source_path, &dest_path, wal).await
        }
        (a, b) if std::mem::discriminant(&a) == std::mem::discriminant(&b) => {
            Err(CoworkerError::Store(format!(
                "source and destination are both {a:?}; pick json→sqlite or sqlite→json"
            )))
        }
        _ => Err(CoworkerError::Store("unsupported migrate direction".into())),
    }
}

async fn migrate_json_to_sqlite(
    json_root: &Path,
    sqlite_path: &Path,
    wal: bool,
) -> Result<MigrateStats> {
    if !json_root.is_dir() {
        return Err(CoworkerError::Store(format!(
            "json source not found: {}",
            json_root.display()
        )));
    }
    if sqlite_path.exists() {
        return Err(CoworkerError::Store(format!(
            "destination sqlite file already exists: {}",
            sqlite_path.display()
        )));
    }
    let dst = SqliteStore::open(sqlite_path.to_path_buf(), wal)?;
    import_json_tree(json_root, &dst).await
}

async fn migrate_sqlite_to_json(
    sqlite_path: &Path,
    json_root: &Path,
    wal: bool,
) -> Result<MigrateStats> {
    if !sqlite_path.is_file() {
        return Err(CoworkerError::Store(format!(
            "sqlite source not found: {}",
            sqlite_path.display()
        )));
    }
    if json_root.exists() && dir_not_empty(json_root)? {
        return Err(CoworkerError::Store(format!(
            "destination json directory is not empty: {}",
            json_root.display()
        )));
    }
    let src = SqliteStore::open(sqlite_path.to_path_buf(), wal)?;
    let dst = JsonStore::open(json_root.to_path_buf())?;
    export_store_to_json(&src, &dst).await
}

async fn import_json_tree(json_root: &Path, dst: &dyn crate::store::Store) -> Result<MigrateStats> {
    use std::collections::HashMap;

    use crate::store::{Approval, BackportQueueItem, ChatMessage};

    let mut stats = MigrateStats::default();

    let pending_path = json_root.join("approvals/pending.json");
    if pending_path.exists() {
        for item in read_json_file::<Vec<Approval>>(&pending_path)? {
            dst.push_approval(&item).await?;
            stats.approvals += 1;
        }
    }

    let backport_path = json_root.join("backport_queue/items.json");
    if backport_path.exists() {
        let items: HashMap<String, BackportQueueItem> = read_json_file(&backport_path)?;
        for item in items.into_values() {
            dst.upsert_backport_queue(&item).await?;
            stats.backport_items += 1;
        }
    }

    let messages_dir = json_root.join("chat/messages");
    if messages_dir.is_dir() {
        for entry in fs::read_dir(&messages_dir)? {
            for msg in read_jsonl_file::<ChatMessage>(&entry?.path())? {
                dst.append_chat_message(&msg).await?;
                stats.chat_messages += 1;
            }
        }
    }

    Ok(stats)
}

async fn export_store_to_json(
    src: &dyn crate::store::Store,
    dst: &dyn crate::store::Store,
) -> Result<MigrateStats> {
    let mut stats = MigrateStats::default();

    for item in src.list_pending_approvals().await? {
        dst.push_approval(&item).await?;
        stats.approvals += 1;
    }

    for item in src.list_backport_queue(None).await? {
        dst.upsert_backport_queue(&item).await?;
        stats.backport_items += 1;
    }

    Ok(stats)
}

fn dir_not_empty(path: &Path) -> Result<bool> {
    if !path.is_dir() {
        return Ok(false);
    }
    Ok(fs::read_dir(path)?.next().is_some())
}

fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let data = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}

fn read_jsonl_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let raw = fs::read_to_string(path)?;
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(CoworkerError::from))
        .collect()
}
