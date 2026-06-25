use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct CheckRollup {
    #[serde(rename = "__typename")]
    pub typename: Option<String>,
    pub name: Option<String>,
    pub context: Option<String>,
    pub status: Option<String>,
    pub conclusion: Option<String>,
    pub state: Option<String>,
    #[serde(default, rename = "detailsUrl")]
    pub details_url: Option<String>,
    #[serde(default, rename = "targetUrl")]
    pub target_url: Option<String>,
}

pub fn check_display_name(c: &CheckRollup) -> String {
    if let Some(ref n) = c.name {
        if !n.is_empty() {
            return n.clone();
        }
    }
    c.context.clone().unwrap_or_default()
}

pub fn check_details_url(c: &CheckRollup) -> String {
    if let Some(u) = c
        .details_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return u.to_string();
    }
    c.target_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_string()
}

pub fn check_verdict(c: &CheckRollup) -> String {
    if c.typename.as_deref() == Some("CheckRun") {
        if c.status.as_deref() != Some("COMPLETED") {
            return "PENDING".into();
        }
        return c.conclusion.clone().unwrap_or_default();
    }
    c.state.clone().unwrap_or_default()
}

pub fn tally_checks(checks: &[CheckRollup]) -> (i32, i32, i32) {
    let mut pass = 0i32;
    let mut fail = 0i32;
    let mut pending = 0i32;
    for c in checks {
        match check_verdict(c).to_ascii_uppercase().as_str() {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => pass += 1,
            "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED"
            | "STARTUP_FAILURE" => fail += 1,
            _ => pending += 1,
        }
    }
    (pass, fail, pending)
}

pub fn ci_state(checks: &[CheckRollup]) -> String {
    if checks.is_empty() {
        return "none".into();
    }
    let (pass, fail, pending) = tally_checks(checks);
    if fail > 0 {
        format!("failing({fail})")
    } else if pending > 0 {
        "pending".into()
    } else if pass > 0 {
        "passing".into()
    } else {
        "none".into()
    }
}

pub fn review_state(decision: &str) -> String {
    match decision.to_ascii_uppercase().as_str() {
        "APPROVED" => "approved".into(),
        "CHANGES_REQUESTED" => "changes-requested".into(),
        "REVIEW_REQUIRED" => "review-required".into(),
        _ => "none".into(),
    }
}

pub fn mergeable_state(mergeable: &str, fail: i32, pending: i32) -> String {
    match mergeable.to_ascii_uppercase().as_str() {
        "CONFLICTING" => "no (merge conflicts)".into(),
        "UNKNOWN" | "" => "unknown (still computing)".into(),
        _ => {
            if fail > 0 {
                "no (CI failing)".into()
            } else if pending > 0 {
                "not yet (CI pending)".into()
            } else {
                "yes".into()
            }
        }
    }
}

pub fn format_external_check_summary(checks: &[CheckRollup]) -> String {
    let mut lines = Vec::new();
    for c in checks {
        if c.typename.as_deref() != Some("StatusContext") {
            continue;
        }
        let name = check_display_name(c);
        if name.is_empty() {
            continue;
        }
        lines.push(format!(
            "  - {}: {}",
            name,
            check_verdict(c).to_ascii_lowercase()
        ));
    }
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "External checks (not GitHub Actions — inspect PR page, do not call ci_get_failed_logs):\n{}\n",
        lines.join("\n")
    )
}

pub fn short_sha(sha: &str) -> String {
    if sha.chars().count() > 7 {
        sha.chars().take(7).collect()
    } else {
        sha.to_string()
    }
}

pub fn is_check_failing(c: &CheckRollup) -> bool {
    matches!(
        check_verdict(c).to_ascii_uppercase().as_str(),
        "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED"
    )
}

pub fn external_checks_failing(checks: &[CheckRollup]) -> bool {
    checks
        .iter()
        .filter(|c| c.typename.as_deref() == Some("StatusContext"))
        .any(is_check_failing)
}

pub fn pending_check_summary(checks: &[CheckRollup]) -> String {
    let mut lines = Vec::new();
    for c in checks {
        let v = check_verdict(c).to_ascii_uppercase();
        if matches!(v.as_str(), "SUCCESS" | "NEUTRAL" | "SKIPPED") {
            continue;
        }
        if matches!(v.as_str(), "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED") {
            continue;
        }
        let name = check_display_name(c);
        if name.is_empty() {
            continue;
        }
        lines.push(format!("  - {}: {}", name, v.to_ascii_lowercase()));
    }
    if lines.is_empty() {
        return String::new();
    }
    format!("Pending checks:\n{}\n", lines.join("\n"))
}
