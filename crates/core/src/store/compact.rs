use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};

use crate::config::StorageBackend;
use crate::error::{CoworkerError, Result};
use crate::store::AuditEntry;

use super::json::JsonStore;
use super::sqlite::SqliteStore;

#[derive(Debug, Clone)]
pub struct CompactOptions {
    pub audit_days: u32,
    /// When true, count what would be pruned but do not delete anything.
    pub dry_run: bool,
}

impl Default for CompactOptions {
    fn default() -> Self {
        Self {
            audit_days: 90,
            dry_run: false,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CompactStats {
    pub audit_entries_removed: u32,
    pub audit_files_removed: u32,
    /// Legacy digest / PR snapshot / triage transcript files removed.
    pub legacy_artifacts_removed: u32,
    /// Legacy batch-workflow run files/rows removed from older stores.
    pub legacy_workflow_runs_removed: u32,
}

pub fn compact(
    backend: StorageBackend,
    path: PathBuf,
    wal: bool,
    opts: &CompactOptions,
) -> Result<CompactStats> {
    match backend {
        StorageBackend::Json => compact_json(&path, opts),
        StorageBackend::Sqlite => compact_sqlite(&path, wal, opts),
    }
}

pub fn compact_json(root: &Path, opts: &CompactOptions) -> Result<CompactStats> {
    if !root.is_dir() {
        return Err(CoworkerError::Store(format!(
            "json store not found: {}",
            root.display()
        )));
    }
    let _ = JsonStore::open(root.to_path_buf())?;
    let (audit_entries_removed, audit_files_removed) =
        prune_json_audit(root, opts.audit_days, opts.dry_run)?;
    Ok(CompactStats {
        audit_entries_removed,
        audit_files_removed,
        legacy_artifacts_removed: purge_json_legacy_artifacts(root, opts.dry_run)?,
        legacy_workflow_runs_removed: purge_json_workflow_runs(root, opts.dry_run)?,
    })
}

pub fn compact_sqlite(path: &Path, wal: bool, opts: &CompactOptions) -> Result<CompactStats> {
    if !path.is_file() {
        return Err(CoworkerError::Store(format!(
            "sqlite store not found: {}",
            path.display()
        )));
    }
    let store = SqliteStore::open(path.to_path_buf(), wal)?;
    Ok(CompactStats {
        audit_entries_removed: prune_sqlite_audit(&store, opts.audit_days, opts.dry_run)?,
        audit_files_removed: 0,
        legacy_artifacts_removed: purge_sqlite_legacy_artifacts(&store, opts.dry_run)?,
        legacy_workflow_runs_removed: purge_sqlite_workflow_runs(&store, opts.dry_run)?,
    })
}

fn audit_cutoff(days: u32) -> DateTime<Utc> {
    Utc::now() - Duration::days(i64::from(days))
}

fn prune_json_audit(root: &Path, audit_days: u32, dry_run: bool) -> Result<(u32, u32)> {
    let dir = root.join("audit");
    if !dir.is_dir() {
        return Ok((0, 0));
    }
    let cutoff = audit_cutoff(audit_days);
    let mut removed = 0u32;
    let mut files_removed = 0u32;
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "jsonl") {
            continue;
        }
        let raw = fs::read_to_string(&path)?;
        let mut kept = Vec::new();
        for line in raw.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<AuditEntry>(line) {
                Ok(entry) if entry.ts >= cutoff => kept.push(line.to_string()),
                Ok(_) => removed += 1,
                Err(_) => kept.push(line.to_string()),
            }
        }
        if kept.is_empty() {
            files_removed += 1;
            if !dry_run {
                fs::remove_file(&path)?;
            }
        } else if !dry_run {
            let mut out = kept.join("\n");
            out.push('\n');
            fs::write(&path, out)?;
        }
    }
    Ok((removed, files_removed))
}

fn purge_json_legacy_artifacts(root: &Path, dry_run: bool) -> Result<u32> {
    let mut removed = 0u32;
    for sub in ["digests", "pr_snapshots", "transcripts"] {
        let dir = root.join(sub);
        if !dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_file() {
                removed += 1;
                if !dry_run {
                    fs::remove_file(path)?;
                }
            }
        }
        if !dry_run {
            let _ = fs::remove_dir(&dir);
        }
    }
    Ok(removed)
}

/// Remove legacy `workflow_runs/*.json` left from pre-3.0 batch workflows.
fn purge_json_workflow_runs(root: &Path, dry_run: bool) -> Result<u32> {
    let dir = root.join("workflow_runs");
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut removed = 0u32;
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        removed += 1;
        if !dry_run {
            fs::remove_file(path)?;
        }
    }
    if removed > 0 && !dry_run {
        let _ = fs::remove_dir(&dir);
    }
    Ok(removed)
}

fn prune_sqlite_audit(store: &SqliteStore, audit_days: u32, dry_run: bool) -> Result<u32> {
    let cutoff = audit_cutoff(audit_days).to_rfc3339();
    store.with_conn(|conn| {
        if dry_run {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM audit_log WHERE ts < ?1",
                [&cutoff],
                |row| row.get(0),
            )?;
            return Ok(n as u32);
        }
        let n = conn.execute("DELETE FROM audit_log WHERE ts < ?1", [&cutoff])?;
        Ok(n as u32)
    })
}

fn purge_sqlite_legacy_artifacts(store: &SqliteStore, dry_run: bool) -> Result<u32> {
    store.with_conn(|conn| {
        let mut total = 0u32;
        for table in ["digests", "pr_snapshots", "transcripts"] {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            if exists == 0 {
                continue;
            }
            if dry_run {
                let n: i64 =
                    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })?;
                total += n as u32;
            } else {
                let n = conn.execute(&format!("DELETE FROM {table}"), [])?;
                total += n as u32;
            }
        }
        Ok(total)
    })
}

fn purge_sqlite_workflow_runs(store: &SqliteStore, dry_run: bool) -> Result<u32> {
    store.with_conn(|conn| {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='workflow_runs'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if exists == 0 {
            return Ok(0);
        }
        if dry_run {
            let n: i64 =
                conn.query_row("SELECT COUNT(*) FROM workflow_runs", [], |row| row.get(0))?;
            return Ok(n as u32);
        }
        let n = conn.execute("DELETE FROM workflow_runs", [])?;
        Ok(n as u32)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::fs::File;
    use std::io::Write;

    fn write_json(path: &Path, value: &impl serde::Serialize) {
        let f = File::create(path).unwrap();
        serde_json::to_writer_pretty(f, value).unwrap();
    }

    #[test]
    fn json_compact_prunes_audit_and_legacy_artifacts() {
        let root = tempfile::tempdir().unwrap();
        JsonStore::open(root.path().to_path_buf()).unwrap();
        let old = audit_cutoff(90) - Duration::days(1);
        let audit_path = root.path().join("audit/2024-01.jsonl");
        fs::create_dir_all(audit_path.parent().unwrap()).unwrap();
        let mut f = File::create(&audit_path).unwrap();
        writeln!(
            f,
            "{}",
            serde_json::to_string(&AuditEntry {
                id: uuid::Uuid::new_v4(),
                ts: old,
                level: "info".into(),
                event: "old".into(),
                message: "stale".into(),
            })
            .unwrap()
        )
        .unwrap();

        fs::create_dir_all(root.path().join("digests")).unwrap();
        for i in 0..3 {
            let date = NaiveDate::from_ymd_opt(2024, 1, i + 1).unwrap();
            write_json(
                &root.path().join(format!("digests/{date}.json")),
                &serde_json::json!({"id": uuid::Uuid::new_v4(), "date": date.to_string()}),
            );
        }

        let stats = compact_json(
            root.path(),
            &CompactOptions {
                audit_days: 90,
                dry_run: false,
            },
        )
        .unwrap();
        assert!(stats.audit_entries_removed >= 1);
        assert_eq!(stats.legacy_artifacts_removed, 3);
        assert!(!root.path().join("digests").exists());
    }
}
