use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, NaiveDate, Utc};

use crate::config::StorageBackend;
use crate::error::{CoworkerError, Result};
use crate::store::{AuditEntry, Digest};

use super::json::JsonStore;
use super::sqlite::SqliteStore;

#[derive(Debug, Clone)]
pub struct CompactOptions {
    pub audit_days: u32,
    pub digest_keep: u32,
    /// When true, count what would be pruned but do not delete anything.
    pub dry_run: bool,
}

impl Default for CompactOptions {
    fn default() -> Self {
        Self {
            audit_days: 90,
            digest_keep: 30,
            dry_run: false,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CompactStats {
    pub audit_entries_removed: u32,
    pub audit_files_removed: u32,
    pub digests_removed: u32,
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
        digests_removed: prune_json_digests(root, opts.digest_keep, opts.dry_run)?,
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
        digests_removed: prune_sqlite_digests(&store, opts.digest_keep, opts.dry_run)?,
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

fn prune_json_digests(root: &Path, digest_keep: u32, dry_run: bool) -> Result<u32> {
    let dir = root.join("digests");
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut files: Vec<(NaiveDate, PathBuf)> = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let digest: Digest = serde_json::from_str(&fs::read_to_string(&path)?)?;
        files.push((digest.date, path));
    }
    files.sort_by_key(|(date, _)| std::cmp::Reverse(*date));
    let mut removed = 0u32;
    for (_, path) in files.into_iter().skip(digest_keep as usize) {
        removed += 1;
        if !dry_run {
            fs::remove_file(path)?;
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
                |r| r.get(0),
            )?;
            return Ok(n as u32);
        }
        let n = conn.execute("DELETE FROM audit_log WHERE ts < ?1", [&cutoff])?;
        Ok(n as u32)
    })
}

fn prune_sqlite_digests(store: &SqliteStore, digest_keep: u32, dry_run: bool) -> Result<u32> {
    store.with_conn(|conn| {
        let mut stmt = conn.prepare("SELECT date FROM digests ORDER BY date DESC LIMIT ?1")?;
        let keep: Vec<String> = stmt
            .query_map([digest_keep as i64], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        if keep.is_empty() {
            return Ok(0);
        }
        let placeholders = keep.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let params: Vec<&dyn rusqlite::ToSql> =
            keep.iter().map(|d| d as &dyn rusqlite::ToSql).collect();
        if dry_run {
            let sql = format!("SELECT COUNT(*) FROM digests WHERE date NOT IN ({placeholders})");
            let n: i64 = conn.query_row(&sql, params.as_slice(), |r| r.get(0))?;
            return Ok(n as u32);
        }
        let sql = format!("DELETE FROM digests WHERE date NOT IN ({placeholders})");
        let n = conn.execute(&sql, params.as_slice())?;
        Ok(n as u32)
    })
}

fn purge_sqlite_workflow_runs(store: &SqliteStore, dry_run: bool) -> Result<u32> {
    store.with_conn(|conn| {
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='workflow_runs'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        if !exists {
            return Ok(0);
        }
        if dry_run {
            let n: i64 = conn.query_row("SELECT COUNT(*) FROM workflow_runs", [], |r| r.get(0))?;
            return Ok(n as u32);
        }
        let n = conn.execute("DELETE FROM workflow_runs", [])?;
        Ok(n as u32)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use chrono::TimeZone;
    use uuid::Uuid;

    fn sample_digest(date: NaiveDate) -> Digest {
        use crate::store::DigestSummary;
        Digest {
            id: Uuid::new_v4(),
            date,
            summary: DigestSummary {
                needs_attention: 0,
                ignorable: 0,
                flaky_candidates: 0,
                policy_gates: 0,
                duration_secs: 0.0,
                complete: true,
            },
            body_md: "# test".into(),
            created_at: Utc::now(),
            skill: None,
        }
    }

    fn write_json<T: serde::Serialize>(path: &Path, value: &T) {
        let data = serde_json::to_vec_pretty(value).unwrap();
        fs::write(path, data).unwrap();
    }

    fn append_audit(root: &Path, entry: &AuditEntry) {
        let month = entry.ts.format("%Y-%m").to_string();
        let path = root.join(format!("audit/{month}.jsonl"));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut line = serde_json::to_string(entry).unwrap();
        line.push('\n');
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(line.as_bytes())
            .unwrap();
    }

    use std::io::Write;

    #[test]
    fn json_compact_prunes_audit_digests_and_legacy_workflow_runs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        JsonStore::open(root.to_path_buf()).unwrap();

        let old_ts = Utc.with_ymd_and_hms(2020, 1, 15, 12, 0, 0).unwrap();
        let recent_ts = Utc::now() - Duration::days(1);
        append_audit(
            root,
            &AuditEntry {
                id: Uuid::new_v4(),
                ts: old_ts,
                level: "info".into(),
                event: "old".into(),
                message: "drop me".into(),
            },
        );
        append_audit(
            root,
            &AuditEntry {
                id: Uuid::new_v4(),
                ts: recent_ts,
                level: "info".into(),
                event: "new".into(),
                message: "keep me".into(),
            },
        );

        for day in 1u32..=9 {
            let date = NaiveDate::from_ymd_opt(2026, 1, day).unwrap();
            let digest = sample_digest(date);
            write_json(&root.join(format!("digests/{date}.json")), &digest);
        }

        let legacy_dir = root.join("workflow_runs");
        fs::create_dir_all(&legacy_dir).unwrap();
        write_json(
            &legacy_dir.join("old-run.json"),
            &serde_json::json!({"id": Uuid::new_v4(), "workflow_id": "daily-work"}),
        );
        write_json(
            &legacy_dir.join("other-run.json"),
            &serde_json::json!({"id": Uuid::new_v4(), "workflow_id": "daily-work"}),
        );

        let stats = compact_json(
            root,
            &CompactOptions {
                audit_days: 90,
                digest_keep: 3,
                dry_run: false,
            },
        )
        .unwrap();

        assert_eq!(stats.audit_entries_removed, 1);
        assert_eq!(stats.digests_removed, 6);
        assert_eq!(stats.legacy_workflow_runs_removed, 2);
        assert!(!legacy_dir.exists());

        assert!(!root.join("audit/2020-01.jsonl").exists());

        let current_month = recent_ts.format("%Y-%m").to_string();
        let audit_raw =
            fs::read_to_string(root.join("audit").join(format!("{current_month}.jsonl"))).unwrap();
        assert!(audit_raw.contains("keep me"));
        assert!(!audit_raw.contains("drop me"));

        let remaining_digests: Vec<_> = fs::read_dir(root.join("digests"))
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(remaining_digests.len(), 3);
    }

    #[test]
    fn json_compact_removes_empty_audit_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        JsonStore::open(root.to_path_buf()).unwrap();

        let old_ts = Utc.with_ymd_and_hms(2019, 6, 1, 0, 0, 0).unwrap();
        append_audit(
            root,
            &AuditEntry {
                id: Uuid::new_v4(),
                ts: old_ts,
                level: "info".into(),
                event: "gone".into(),
                message: "all old".into(),
            },
        );

        let stats = compact_json(
            root,
            &CompactOptions {
                audit_days: 30,
                digest_keep: 30,
                dry_run: false,
            },
        )
        .unwrap();

        assert_eq!(stats.audit_entries_removed, 1);
        assert_eq!(stats.audit_files_removed, 1);
        assert!(!root.join("audit/2019-06.jsonl").exists());
    }

    #[test]
    fn sqlite_compact_prunes_tables() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("store.db");
        let store = SqliteStore::open(db_path.clone(), false).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let old_ts = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
            let recent_ts = Utc::now() - Duration::days(1);

            store
                .append_audit(&AuditEntry {
                    id: Uuid::new_v4(),
                    ts: old_ts,
                    level: "info".into(),
                    event: "old".into(),
                    message: "drop".into(),
                })
                .await
                .unwrap();
            store
                .append_audit(&AuditEntry {
                    id: Uuid::new_v4(),
                    ts: recent_ts,
                    level: "info".into(),
                    event: "new".into(),
                    message: "keep".into(),
                })
                .await
                .unwrap();

            for day in 1u32..=5 {
                let date = NaiveDate::from_ymd_opt(2026, 2, day).unwrap();
                store.save_digest(&sample_digest(date)).await.unwrap();
            }

            store
                .with_conn(|conn| {
                    conn.execute(
                        "CREATE TABLE IF NOT EXISTS workflow_runs (
                            id TEXT PRIMARY KEY,
                            workflow_id TEXT NOT NULL,
                            payload_json TEXT NOT NULL
                        )",
                        [],
                    )?;
                    conn.execute(
                        "INSERT INTO workflow_runs (id, workflow_id, payload_json) VALUES (?1,?2,?3)",
                        rusqlite::params![
                            Uuid::new_v4().to_string(),
                            "daily-work",
                            r#"{"summary":"legacy"}"#
                        ],
                    )?;
                    Ok(())
                })
                .unwrap();
        });

        let stats = compact_sqlite(
            &db_path,
            false,
            &CompactOptions {
                audit_days: 90,
                digest_keep: 2,
                dry_run: false,
            },
        )
        .unwrap();

        assert_eq!(stats.audit_entries_removed, 1);
        assert_eq!(stats.digests_removed, 3);
        assert_eq!(stats.legacy_workflow_runs_removed, 1);

        rt.block_on(async {
            let digests = store.list_digests(10).await.unwrap();
            assert_eq!(digests.len(), 2);
        });
    }
}
