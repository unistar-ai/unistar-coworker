use serde::Deserialize;
use serde_json::Value;

use super::args::{optional_u32, require_str};
use super::exec::GhExec;
use crate::error::Result;

const DEFAULT_ALERT_LIMIT: u32 = 20;

#[derive(Debug, Deserialize)]
struct DependabotAlertRow {
    number: u32,
    #[serde(default)]
    state: String,
    #[serde(rename = "security_advisory")]
    security_advisory: SecurityAdvisory,
}

#[derive(Debug, Deserialize)]
struct SecurityAdvisory {
    #[serde(default)]
    severity: String,
    #[serde(default)]
    summary: String,
}

pub async fn alert_list_open(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let mut limit = optional_u32(args, "limit", DEFAULT_ALERT_LIMIT);
    if limit == 0 {
        limit = DEFAULT_ALERT_LIMIT;
    }
    let alerts = fetch_open_dependabot_alerts(exec, &repo, limit).await?;
    if alerts.is_empty() {
        return Ok(format!("No open Dependabot alerts in {repo}."));
    }
    let mut lines = vec![format!(
        "{} open Dependabot alert(s) in {repo}:",
        alerts.len()
    )];
    for a in &alerts {
        let sev = a.security_advisory.severity.to_ascii_uppercase();
        let mut summary = a.security_advisory.summary.clone();
        if summary.chars().count() > 120 {
            summary = format!("{}…", summary.chars().take(120).collect::<String>());
        }
        lines.push(format!("#{}  {sev}  {summary}  [{}]", a.number, a.state));
    }
    Ok(lines.join("\n"))
}

pub async fn alert_summarize_open(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let mut limit = optional_u32(args, "limit", 100);
    if limit == 0 {
        limit = 100;
    }
    let alerts = fetch_open_dependabot_alerts(exec, &repo, limit).await?;
    if alerts.is_empty() {
        return Ok(format!("No open Dependabot alerts in {repo}."));
    }
    Ok(format_alert_severity_summary(&repo, &alerts))
}

async fn fetch_open_dependabot_alerts(
    exec: &GhExec,
    repo: &str,
    limit: u32,
) -> Result<Vec<DependabotAlertRow>> {
    let jq =
        format!(".[] | {{number, state, security_advisory: {{severity, summary}}}} | .[0:{limit}]");
    let path = format!("repos/{repo}/dependabot/alerts");
    let gh_args = ["api", &path, "-f", "state=open", "--paginate", "--jq", &jq];
    let res = exec.run_retry(&gh_args).await;
    let raw = GhExec::into_result(
        res,
        "failed to list dependabot alerts (requires repo admin or security permission)",
    )?;
    let raw = raw.trim();
    if raw.is_empty() || raw == "[]" {
        return Ok(Vec::new());
    }
    if let Ok(alerts) = serde_json::from_str::<Vec<DependabotAlertRow>>(raw) {
        return Ok(alerts);
    }
    let mut alerts = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(row) = serde_json::from_str::<DependabotAlertRow>(line) {
            alerts.push(row);
        }
    }
    Ok(alerts)
}

fn format_alert_severity_summary(repo: &str, alerts: &[DependabotAlertRow]) -> String {
    let order = ["critical", "high", "medium", "low", "unknown"];
    let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut by_sev: std::collections::HashMap<String, Vec<&DependabotAlertRow>> =
        std::collections::HashMap::new();
    for a in alerts {
        let mut sev = a.security_advisory.severity.trim().to_ascii_lowercase();
        if sev.is_empty() {
            sev = "unknown".into();
        }
        *counts.entry(sev.clone()).or_default() += 1;
        by_sev.entry(sev).or_default().push(a);
    }
    let mut lines = vec![format!(
        "Dependabot alert summary for {repo} ({} open):",
        alerts.len()
    )];
    for sev in order {
        let n = counts.get(sev).copied().unwrap_or(0);
        if n == 0 {
            continue;
        }
        lines.push(format!("{}: {n}", sev.to_ascii_uppercase()));
        if let Some(mut top) = by_sev.get(sev).cloned() {
            top.truncate(3);
            for a in top {
                let mut summary = a.security_advisory.summary.clone();
                if summary.chars().count() > 80 {
                    summary = format!("{}…", summary.chars().take(80).collect::<String>());
                }
                lines.push(format!("  #{}  {summary}", a.number));
            }
        }
    }
    lines.push("Next: alert_list_open for full list.".into());
    lines.join("\n")
}
