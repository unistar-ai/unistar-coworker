use crate::agent::tool_catalog;
use crate::error::{CoworkerError, Result};

pub const META_TOOLS: &[&str] = &[
    "tool_list",
    "tool_list_category",
    "tool_search",
    "tool_describe",
    "tool_call",
];

pub fn is_meta_tool(name: &str) -> bool {
    META_TOOLS.contains(&name)
}

pub fn tool_list() -> String {
    let names = tool_catalog::list_github_tool_names();
    let mut out = format!("{} tool(s) available:\n", names.len());
    for name in names {
        let blurb = brief(tool_catalog::tool_blurb_for_name(&name));
        out.push_str(&format!("[{}] {name} — {blurb}\n", tool_category(&name)));
    }
    out.trim_end().to_string()
}

pub fn tool_list_category(category: &str) -> Result<String> {
    let want = normalize_category(category).ok_or_else(|| {
        CoworkerError::Workflow(format!(
            "unknown category {category:?} — use CI, PR, Repo, Issue, Security, Release, Policy, Backport, Notify, or Event"
        ))
    })?;
    let mut n = 0usize;
    let mut body = String::new();
    for name in tool_catalog::list_github_tool_names() {
        if tool_category(&name) != want {
            continue;
        }
        n += 1;
        let blurb = brief(tool_catalog::tool_blurb_for_name(&name));
        body.push_str(&format!("[{want}] {name} — {blurb}\n"));
    }
    if n == 0 {
        return Err(CoworkerError::Workflow(format!(
            "no tools in category {category:?}"
        )));
    }
    Ok(format!("{n} tool(s) in [{want}]:\n{body}").trim_end().to_string())
}

pub fn tool_search(query: &str, limit: usize) -> Result<String> {
    let limit = limit.clamp(1, 15);
    let tokens = search_tokens(query);
    if tokens.is_empty() {
        return Err(CoworkerError::Workflow(
            "query is empty — pass keywords like \"pr ci\" or \"merge blockers\"".into(),
        ));
    }
    let mut scored: Vec<(i32, String)> = Vec::new();
    for name in tool_catalog::list_github_tool_names() {
        let blurb = tool_catalog::tool_blurb_for_name(&name);
        let score = score_tool_search(&name, blurb, &tokens);
        if score > 0 {
            scored.push((
                score,
                format!("[{}] {name} — {}", tool_category(&name), brief(blurb)),
            ));
        }
    }
    scored.sort_by_key(|b| std::cmp::Reverse(b.0));
    scored.truncate(limit);
    if scored.is_empty() {
        return Err(CoworkerError::Workflow(format!(
            "no tools matched {query:?} — try tool_list_category or tool_list"
        )));
    }
    let mut out = format!("{} match(es) for {query:?}:\n", scored.len());
    for (_, line) in scored {
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out.trim_end().to_string())
}

pub fn tool_describe(name: &str) -> Result<String> {
    if !tool_catalog::ToolCatalog::new().is_known_chat_tool(name) {
        return Err(CoworkerError::Workflow(format!("unknown tool {name:?}")));
    }
    let blurb = tool_catalog::tool_blurb_for_name(name);
    let (req, opt) = tool_catalog::tool_fields_for_name(name);
    let mut out = format!("{name}\n{blurb}\n\nParameters (JSON Schema):\n{{\n  \"type\": \"object\",\n  \"properties\": {{");
    for f in req.iter().chain(opt.iter()) {
        out.push_str(&format!("\n    \"{f}\": {{\"type\": \"string\"}},"));
    }
    out.push_str("\n  },\n  \"required\": [");
    for (i, f) in req.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("\"{f}\""));
    }
    out.push_str("]\n}");
    Ok(out)
}

fn brief(desc: &str) -> String {
    if let Some(dot) = desc.find(". ") {
        desc[..dot + 1].to_string()
    } else {
        desc.to_string()
    }
}

fn tool_category(name: &str) -> &'static str {
    if name.starts_with("ci_") {
        return "CI";
    }
    if name.starts_with("pr_") {
        return "PR";
    }
    if name.starts_with("repo_") {
        return "Repo";
    }
    if name.starts_with("issue_") {
        return "Issue";
    }
    if name.starts_with("alert_") {
        return "Security";
    }
    if name.starts_with("notify_") {
        return "Notify";
    }
    if name.starts_with("event_") {
        return "Event";
    }
    if name.starts_with("policy_") {
        return "Policy";
    }
    if name.starts_with("backport_") {
        return "Backport";
    }
    if name.starts_with("release_") {
        return "Release";
    }
    "Tool"
}

fn normalize_category(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "CI" => Some("CI"),
        "PR" => Some("PR"),
        "REPO" => Some("Repo"),
        "ISSUE" => Some("Issue"),
        "SECURITY" => Some("Security"),
        "RELEASE" => Some("Release"),
        "POLICY" => Some("Policy"),
        "BACKPORT" => Some("Backport"),
        "NOTIFY" => Some("Notify"),
        "EVENT" => Some("Event"),
        "TOOL" => Some("Tool"),
        _ => None,
    }
}

fn search_tokens(query: &str) -> Vec<String> {
    query
        .to_ascii_lowercase()
        .split(|c: char| c.is_whitespace() || c == '_' || c == '-' || c == ',')
        .filter(|t| t.len() >= 2)
        .map(str::to_string)
        .collect()
}

fn score_tool_search(name: &str, desc: &str, tokens: &[String]) -> i32 {
    let low_name = name.to_ascii_lowercase();
    let low_desc = desc.to_ascii_lowercase();
    let mut score = 0i32;
    for tok in tokens {
        if low_name.contains(tok) {
            score += 10 + tok.len() as i32;
        }
        if low_desc.contains(tok) {
            score += 4 + (tok.len() as i32 / 2);
        }
    }
    score
}
