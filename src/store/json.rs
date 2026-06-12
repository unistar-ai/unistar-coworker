use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::error::{CoworkerError, Result};
use crate::store::{
    Approval, ApprovalStatus, AuditEntry, BackportQueueItem, Digest, DigestMeta, FlakyIncident,
    FlakyQuery, FlakyTestRollup, PrSnapshot, RerunOutcome, Store, WorkflowRun,
};
use async_trait::async_trait;

#[derive(Debug)]
pub struct JsonStore {
    root: PathBuf,
}

impl JsonStore {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join("digests"))?;
        fs::create_dir_all(root.join("pr_snapshots"))?;
        fs::create_dir_all(root.join("approvals"))?;
        fs::create_dir_all(root.join("flaky"))?;
        fs::create_dir_all(root.join("audit"))?;
        fs::create_dir_all(root.join("workflow_runs"))?;
        fs::create_dir_all(root.join("backport_queue"))?;

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
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(line.as_bytes())?;
        Ok(())
    }

    fn repo_file(repo: &str) -> String {
        repo.replace('/', "__")
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let data = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}

#[async_trait]
impl Store for JsonStore {
    async fn save_digest(&self, digest: &Digest) -> Result<()> {
        let path = self
            .root
            .join("digests")
            .join(format!("{}.json", digest.date));
        Self::write_json(&path, digest)
    }

    async fn latest_digest(&self) -> Result<Option<Digest>> {
        let dir = self.root.join("digests");
        let mut files: Vec<_> = fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort_by_key(|e| e.file_name());
        if let Some(last) = files.pop() {
            return Ok(Some(read_json(&last.path())?));
        }
        Ok(None)
    }

    async fn list_digests(&self, limit: usize) -> Result<Vec<DigestMeta>> {
        let dir = self.root.join("digests");
        let mut metas = Vec::new();
        if !dir.exists() {
            return Ok(metas);
        }
        let mut files: Vec<_> = fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort_by_key(|e| e.file_name());
        for entry in files.into_iter().rev().take(limit) {
            let digest: Digest = read_json(&entry.path())?;
            metas.push(digest.meta());
        }
        Ok(metas)
    }

    async fn upsert_pr_snapshot(&self, snap: &PrSnapshot) -> Result<()> {
        let path = self
            .root
            .join("pr_snapshots")
            .join(format!("{}.json", Self::repo_file(&snap.repo)));
        let mut map: HashMap<u32, PrSnapshot> = if path.exists() {
            read_json(&path)?
        } else {
            HashMap::new()
        };
        map.insert(snap.number, snap.clone());
        Self::write_json(&path, &map)
    }

    async fn list_pr_snapshots(&self, repo: Option<&str>) -> Result<Vec<PrSnapshot>> {
        let dir = self.root.join("pr_snapshots");
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if repo.is_some_and(|r| !entry.file_name().to_string_lossy().contains(&Self::repo_file(r)))
            {
                continue;
            }
            let map: HashMap<u32, PrSnapshot> = read_json(&entry.path())?;
            out.extend(map.into_values());
        }
        out.sort_by_key(|b| std::cmp::Reverse(b.fetched_at));
        Ok(out)
    }

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

    async fn append_audit(&self, entry: &AuditEntry) -> Result<()> {
        let month = entry.ts.format("%Y-%m").to_string();
        let path = self.root.join(format!("audit/{month}.jsonl"));
        Self::append_jsonl(&path, entry)
    }

    async fn record_flaky_incident(&self, incident: &FlakyIncident) -> Result<()> {
        let path = self.root.join("flaky/incidents.jsonl");
        Self::append_jsonl(&path, incident)?;
        // Interior mutability workaround: re-read rollups, update, save
        let mut rollups: HashMap<String, FlakyTestRollup> =
            read_json(&self.root.join("flaky/tests.json")).unwrap_or_default();
        let entry = rollups
            .entry(incident.fingerprint.clone())
            .or_insert_with(|| FlakyTestRollup {
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
            });
        entry.last_seen = incident.ts;
        entry.incident_count += 1;
        entry.last_error_signature = incident.log_excerpt.chars().take(200).collect();
        Self::write_json(&self.root.join("flaky/tests.json"), &rollups)
    }

    async fn update_flaky_rerun(&self, incident_id: &Uuid, outcome: RerunOutcome) -> Result<()> {
        let path = self.root.join("flaky/incidents.jsonl");
        if !path.exists() {
            return Err(CoworkerError::Store(format!(
                "incident {incident_id} not found"
            )));
        }
        let raw = fs::read_to_string(&path)?;
        let mut incidents: Vec<FlakyIncident> = raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let idx = incidents
            .iter()
            .position(|i| &i.id == incident_id)
            .ok_or_else(|| CoworkerError::Store(format!("incident {incident_id} not found")))?;
        incidents[idx].rerun_outcome = Some(outcome);

        let mut out = String::new();
        for incident in &incidents {
            out.push_str(&serde_json::to_string(incident)?);
            out.push('\n');
        }
        fs::write(&path, out)?;

        let incident = &incidents[idx];
        let mut rollups: HashMap<String, FlakyTestRollup> =
            read_json(&self.root.join("flaky/tests.json")).unwrap_or_default();
        if let Some(entry) = rollups.get_mut(&incident.fingerprint) {
            entry.rerun_attempts += 1;
            if outcome == RerunOutcome::Succeeded {
                entry.rerun_successes += 1;
            }
            Self::write_json(&self.root.join("flaky/tests.json"), &rollups)?;
        }
        Ok(())
    }

    async fn list_flaky_tests(&self, q: FlakyQuery) -> Result<Vec<FlakyTestRollup>> {
        let rollups: HashMap<String, FlakyTestRollup> =
            read_json(&self.root.join("flaky/tests.json")).unwrap_or_default();
        let since = q
            .since_days
            .map(|d| Utc::now() - Duration::days(i64::from(d)));
        let mut list: Vec<_> = rollups
            .into_values()
            .filter(|t| q.repo.as_ref().is_none_or(|r| &t.repo == r))
            .filter(|t| since.is_none_or(|s| t.last_seen >= s))
            .collect();
        list.sort_by_key(|b| std::cmp::Reverse(b.incident_count));
        list.truncate(q.limit);
        Ok(list)
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

    async fn start_workflow_run(&self, workflow_id: &str) -> Result<Uuid> {
        let run = WorkflowRun {
            id: Uuid::new_v4(),
            workflow_id: workflow_id.to_string(),
            started_at: Utc::now(),
            finished_at: None,
            error: None,
            summary: None,
        };
        let path = self
            .root
            .join("workflow_runs")
            .join(format!("{}.json", run.id));
        Self::write_json(&path, &run)?;
        Ok(run.id)
    }

    async fn finish_workflow_run(
        &self,
        id: &Uuid,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let path = self.root.join("workflow_runs").join(format!("{id}.json"));
        let mut run: WorkflowRun = read_json(&path)?;
        run.finished_at = Some(Utc::now());
        run.summary = summary.map(str::to_string);
        run.error = error.map(str::to_string);
        Self::write_json(&path, &run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::DigestSummary;
    use chrono::Utc;

    #[tokio::test]
    async fn digest_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::open(dir.path().to_path_buf()).unwrap();
        let digest = Digest {
            id: Uuid::new_v4(),
            date: Utc::now().date_naive(),
            summary: DigestSummary {
                needs_attention: 1,
                ignorable: 2,
                flaky_candidates: 0,
                policy_gates: 0,
                duration_secs: 1.5,
                complete: true,
            },
            body_md: "# Daily".into(),
            created_at: Utc::now(),
        };
        store.save_digest(&digest).await.unwrap();
        let loaded = store.latest_digest().await.unwrap().unwrap();
        assert_eq!(loaded.id, digest.id);
        assert_eq!(loaded.summary.duration_secs, 1.5);
    }
}
