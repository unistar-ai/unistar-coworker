use crate::error::{CoworkerError, Result};
use crate::github::discovery;
use crate::mcp::pool::McpPool;
use crate::mcp::registry::GlobalToolEntry;

pub async fn federated_tool_list(pool: &McpPool) -> String {
    let mut out = String::from("## Built-in (github)\n");
    out.push_str(&discovery::tool_list());
    for section in pool.server_sections_async().await {
        out.push('\n');
        out.push_str(&section);
    }
    out
}

pub async fn federated_tool_search(
    pool: &McpPool,
    query: &str,
    limit: usize,
) -> Result<String> {
    let limit = limit.clamp(1, 15);
    let mut lines: Vec<(i32, String)> = Vec::new();

    if let Ok(github) = discovery::tool_search(query, limit) {
        for line in github.lines().skip(1) {
            if line.trim().is_empty() {
                continue;
            }
            lines.push((100, format!("(github) {line}")));
        }
    }

    let tokens = query
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();

    for entry in pool.registry_entries_async().await {
        let score = score_mcp_entry(&entry, &tokens);
        if score > 0 {
            lines.push((
                score,
                format!(
                    "(mcp:{}) {} — {}",
                    entry.server_id, entry.global_name, entry.remote_name
                ),
            ));
        }
    }

    lines.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    lines.truncate(limit);
    if lines.is_empty() {
        return Err(CoworkerError::Workflow(format!(
            "no tools matched {query:?} — try tool_list"
        )));
    }
    let mut out = format!("{} match(es) for {query:?}:\n", lines.len());
    for (_, line) in lines {
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out.trim_end().to_string())
}

pub async fn federated_tool_describe(pool: &McpPool, name: &str) -> Result<String> {
    if let Some(desc) = pool.describe_tool_async(name).await {
        return Ok(desc);
    }
    discovery::tool_describe(name)
}

fn score_mcp_entry(entry: &GlobalToolEntry, tokens: &[String]) -> i32 {
    let hay = format!("{} {} {}", entry.global_name, entry.remote_name, entry.server_id)
        .to_ascii_lowercase();
    let mut score = 0;
    for token in tokens {
        if hay.contains(token) {
            score += 10;
        }
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_mcp_entry_matches_prefix() {
        let entry = GlobalToolEntry {
            global_name: "slack_post_message".into(),
            server_id: "slack".into(),
            remote_name: "post_message".into(),
            mutating: true,
        };
        assert!(score_mcp_entry(&entry, &["slack".into()]) > 0);
    }
}
