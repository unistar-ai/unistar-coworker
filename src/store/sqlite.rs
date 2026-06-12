//! SQLite store — enable with `cargo build --features sqlite`.

use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::error::{CoworkerError, Result};
use crate::store::{
    Approval, ApprovalStatus, AuditEntry, BackportQueueItem, Digest, DigestMeta, FlakyIncident,
    FlakyQuery, FlakyTestRollup, PrSnapshot, RerunOutcome, Store, WorkflowRun,
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

    fn with_conn<F, T>(&self, f: F) -> Result<T>
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
        CREATE TABLE IF NOT EXISTS digests (
            id TEXT PRIMARY KEY,
            date TEXT NOT NULL,
            summary_json TEXT NOT NULL,
            body_md TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS pr_snapshots (
            repo TEXT NOT NULL,
            pr_number INTEGER NOT NULL,
            snapshot_json TEXT NOT NULL,
            fetched_at TEXT NOT NULL,
            PRIMARY KEY (repo, pr_number)
        );
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
        CREATE TABLE IF NOT EXISTS flaky_incidents (
            id TEXT PRIMARY KEY,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS flaky_tests (
            fingerprint TEXT PRIMARY KEY,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS workflow_runs (
            id TEXT PRIMARY KEY,
            workflow_id TEXT NOT NULL,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS backport_queue (
            id TEXT PRIMARY KEY,
            payload_json TEXT NOT NULL
        );
        ",
    )?;
    Ok(())
}

#[async_trait]
impl Store for SqliteStore {
    async fn save_digest(&self, digest: &Digest) -> Result<()> {
        let digest = digest.clone();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO digests (id, date, summary_json, body_md, created_at) VALUES (?1,?2,?3,?4,?5)",
                params![
                    digest.id.to_string(),
                    digest.date.to_string(),
                    serde_json::to_string(&digest.summary)?,
                    digest.body_md,
                    digest.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    async fn latest_digest(&self) -> Result<Option<Digest>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, date, summary_json, body_md, created_at FROM digests ORDER BY date DESC LIMIT 1",
            )?;
            let mut rows = stmt.query([])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row_to_digest(row)?))
            } else {
                Ok(None)
            }
        })
    }

    async fn list_digests(&self, limit: usize) -> Result<Vec<DigestMeta>> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, date, summary_json, body_md, created_at FROM digests ORDER BY date DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([limit as i64], |row| row_to_digest(row))?;
            let mut metas = Vec::new();
            for row in rows {
                metas.push(row?.meta());
            }
            Ok(metas)
        })
    }

    async fn upsert_pr_snapshot(&self, snap: &PrSnapshot) -> Result<()> {
        let snap = snap.clone();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO pr_snapshots (repo, pr_number, snapshot_json, fetched_at) VALUES (?1,?2,?3,?4)",
                params![
                    snap.repo,
                    snap.number,
                    serde_json::to_string(&snap)?,
                    snap.fetched_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    async fn list_pr_snapshots(&self, repo: Option<&str>) -> Result<Vec<PrSnapshot>> {
        let repo = repo.map(str::to_string);
        self.with_conn(move |conn| {
            let mut out = Vec::new();
            if let Some(r) = repo {
                let mut stmt =
                    conn.prepare("SELECT snapshot_json FROM pr_snapshots WHERE repo = ?1")?;
                let rows = stmt.query_map([&r], |row| row.get::<_, String>(0))?;
                for row in rows {
                    out.push(serde_json::from_str(&row?)?);
                }
            } else {
                let mut stmt = conn.prepare("SELECT snapshot_json FROM pr_snapshots")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                for row in rows {
                    out.push(serde_json::from_str(&row?)?);
                }
            }
            Ok(out)
        })
    }

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
            let mut stmt =
                conn.prepare("SELECT payload_json FROM approvals WHERE id = ?1 AND status = 'pending'")?;
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

    async fn record_flaky_incident(&self, incident: &FlakyIncident) -> Result<()> {
        let incident = incident.clone();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO flaky_incidents (id, payload_json) VALUES (?1,?2)",
                params![incident.id.to_string(), serde_json::to_string(&incident)?],
            )?;
            upsert_flaky_rollup(conn, &incident)?;
            Ok(())
        })
    }

    async fn update_flaky_rerun(&self, incident_id: &Uuid, outcome: RerunOutcome) -> Result<()> {
        let incident_id = *incident_id;
        self.with_conn(move |conn| {
            let mut stmt =
                conn.prepare("SELECT payload_json FROM flaky_incidents WHERE id = ?1")?;
            let mut rows = stmt.query([incident_id.to_string()])?;
            let row = rows
                .next()?
                .ok_or_else(|| CoworkerError::Store(format!("incident {incident_id} not found")))?;
            let mut incident: FlakyIncident = serde_json::from_str(&row.get::<_, String>(0)?)?;
            incident.rerun_outcome = Some(outcome);
            conn.execute(
                "UPDATE flaky_incidents SET payload_json = ?1 WHERE id = ?2",
                params![serde_json::to_string(&incident)?, incident_id.to_string()],
            )?;

            let mut stmt = conn.prepare("SELECT payload_json FROM flaky_tests WHERE fingerprint = ?1")?;
            let mut rows = stmt.query([&incident.fingerprint])?;
            if let Some(row) = rows.next()? {
                let mut rollup: FlakyTestRollup = serde_json::from_str(&row.get::<_, String>(0)?)?;
                rollup.rerun_attempts += 1;
                if outcome == RerunOutcome::Succeeded {
                    rollup.rerun_successes += 1;
                }
                conn.execute(
                    "UPDATE flaky_tests SET payload_json = ?1 WHERE fingerprint = ?2",
                    params![
                        serde_json::to_string(&rollup)?,
                        incident.fingerprint.clone()
                    ],
                )?;
            }
            Ok(())
        })
    }

    async fn list_flaky_tests(&self, q: FlakyQuery) -> Result<Vec<FlakyTestRollup>> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare("SELECT payload_json FROM flaky_tests")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let since = q
                .since_days
                .map(|d| Utc::now() - Duration::days(i64::from(d)));
            let mut list: Vec<FlakyTestRollup> = rows
                .filter_map(|r| r.ok())
                .filter_map(|j| serde_json::from_str(&j).ok())
                .filter(|t: &FlakyTestRollup| q.repo.as_ref().is_none_or(|r| &t.repo == r))
                .filter(|t| since.is_none_or(|s| t.last_seen >= s))
                .collect();
            list.sort_by(|a, b| b.incident_count.cmp(&a.incident_count));
            list.truncate(q.limit);
            Ok(list)
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
                .filter(|i| repo.as_ref().is_none_or(|r| &i.repo == r))
                .collect();
            list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            Ok(list)
        })
    }

    async fn start_workflow_run(&self, workflow_id: &str) -> Result<Uuid> {
        let workflow_id = workflow_id.to_string();
        self.with_conn(move |conn| {
            let run = WorkflowRun {
                id: Uuid::new_v4(),
                workflow_id,
                started_at: Utc::now(),
                finished_at: None,
                error: None,
                summary: None,
            };
            conn.execute(
                "INSERT INTO workflow_runs (id, workflow_id, payload_json) VALUES (?1,?2,?3)",
                params![
                    run.id.to_string(),
                    run.workflow_id,
                    serde_json::to_string(&run)?,
                ],
            )?;
            Ok(run.id)
        })
    }

    async fn finish_workflow_run(
        &self,
        id: &Uuid,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let id = *id;
        let summary = summary.map(str::to_string);
        let error = error.map(str::to_string);
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare("SELECT payload_json FROM workflow_runs WHERE id = ?1")?;
            let json: String = stmt.query_row([id.to_string()], |row| row.get(0))?;
            let mut run: WorkflowRun = serde_json::from_str(&json)?;
            run.finished_at = Some(Utc::now());
            run.summary = summary;
            run.error = error;
            conn.execute(
                "UPDATE workflow_runs SET payload_json = ?1 WHERE id = ?2",
                params![serde_json::to_string(&run)?, id.to_string()],
            )?;
            Ok(())
        })
    }
}

fn row_to_digest(row: &rusqlite::Row<'_>) -> rusqlite::Result<Digest> {
    use chrono::NaiveDate;
    let id: String = row.get(0)?;
    let date: String = row.get(1)?;
    let summary_json: String = row.get(2)?;
    let body_md: String = row.get(3)?;
    let created_at: String = row.get(4)?;
    Ok(Digest {
        id: Uuid::parse_str(&id).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
        date: NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
        summary: serde_json::from_str(&summary_json)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
        body_md,
        created_at: created_at
            .parse()
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
    })
}

fn upsert_flaky_rollup(conn: &Connection, incident: &FlakyIncident) -> Result<()> {
    let fp = &incident.fingerprint;
    let existing: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM flaky_tests WHERE fingerprint = ?1",
            [fp],
            |row| row.get(0),
        )
        .ok();
    let mut rollup = if let Some(json) = existing {
        serde_json::from_str(&json)?
    } else {
        FlakyTestRollup {
            fingerprint: incident.fingerprint.clone(),
            repo: incident.repo.clone(),
            workflow: incident.workflow.clone(),
            job: incident.job.clone(),
            test_name: incident.test_name.clone(),
            first_seen: incident.ts,
            last_seen: incident.ts,
            incident_count: 0,
            rerun_attempts: 0,
            rerun_successes: 0,
            last_error_signature: incident.log_excerpt.chars().take(200).collect(),
        }
    };
    rollup.last_seen = incident.ts;
    rollup.incident_count += 1;
    rollup.last_error_signature = incident.log_excerpt.chars().take(200).collect();
    conn.execute(
        "INSERT OR REPLACE INTO flaky_tests (fingerprint, payload_json) VALUES (?1,?2)",
        params![fp, serde_json::to_string(&rollup)?],
    )?;
    Ok(())
}
