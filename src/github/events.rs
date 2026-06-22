use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::args::{optional_str, optional_u32};
use super::exec::GhExec;
use crate::error::Result;

const DEFAULT_EVENT_LIST_LIMIT: u32 = 20;
const MAX_EVENT_LIST_LIMIT: u32 = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EventRecord {
    at: DateTime<Utc>,
    kind: String,
    repo: String,
    summary: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    delivery: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    fingerprint: String,
}

pub async fn event_list_recent(exec: &GhExec, args: &Value) -> Result<String> {
    let _ = exec;
    let repo = optional_str(args, "repo").unwrap_or_default();
    let kind = optional_str(args, "kind").unwrap_or_default();
    let mut limit = optional_u32(args, "limit", DEFAULT_EVENT_LIST_LIMIT);
    if limit == 0 {
        limit = DEFAULT_EVENT_LIST_LIMIT;
    }
    if limit > MAX_EVENT_LIST_LIMIT {
        limit = MAX_EVENT_LIST_LIMIT;
    }

    let events = load_events_from_file()?;
    let total = events.len();
    let filtered = filter_events(&events, &repo, &kind, limit as usize);

    if filtered.is_empty() {
        let mut out = format!("No recent webhook events ({total} stored total).\n");
        out.push_str(
            "hint: run `unistar-mcp http`, point GitHub webhooks to /hooks/github, \
             set GITHUB_WEBHOOK_SECRET when configured on GitHub",
        );
        if let Some(path) = resolve_event_file_path() {
            out.push_str(&format!(
                "\nshared event file: {} (stdio + HTTP processes)",
                path.display()
            ));
        }
        return Ok(out);
    }

    let mut lines = vec![format!(
        "{} recent event(s) (newest first, {total} stored total):",
        filtered.len()
    )];
    for ev in filtered {
        let mut line = format!(
            "{}  {}  {}  {}",
            ev.at.to_rfc3339(),
            ev.kind,
            ev.repo,
            ev.summary
        );
        if !ev.delivery.is_empty() {
            line.push_str(&format!("  delivery:{}", ev.delivery));
        }
        if !ev.fingerprint.is_empty() {
            line.push_str(&format!("  fp:{}", ev.fingerprint));
        }
        lines.push(line);
    }
    Ok(lines.join("\n"))
}

fn filter_events(
    events: &[EventRecord],
    repo: &str,
    kind_prefix: &str,
    limit: usize,
) -> Vec<EventRecord> {
    let repo = repo.trim();
    let kind_prefix = kind_prefix.trim();
    let mut out = Vec::new();
    for ev in events.iter().rev() {
        if !repo.is_empty() && !ev.repo.eq_ignore_ascii_case(repo) {
            continue;
        }
        if !kind_prefix.is_empty() && !ev.kind.starts_with(kind_prefix) {
            continue;
        }
        out.push(ev.clone());
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn load_events_from_file() -> Result<Vec<EventRecord>> {
    let Some(path) = resolve_event_file_path() else {
        return Ok(Vec::new());
    };
    let file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            tracing::warn!("event store: read {}: {e}", path.display());
            return Ok(Vec::new());
        }
    };
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(row) = serde_json::from_str::<EventRecord>(line) {
            events.push(row);
        }
    }
    Ok(events)
}

fn resolve_event_file_path() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("UNISTAR_MCP_EVENT_FILE") {
        let v = v.trim();
        match v.to_ascii_lowercase().as_str() {
            "" | "off" | "memory" | "disabled" | "disable" => return None,
            _ => return Some(PathBuf::from(v)),
        }
    }
    let home = std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())?;
    Some(PathBuf::from(home).join(".cache").join("unistar-mcp").join("events.jsonl"))
}
