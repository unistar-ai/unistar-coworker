use coworker_core::error::{CoworkerError, Result};
use coworker_core::store;

use super::args::{ExportFormat, ExportTarget};

pub(crate) async fn run_export_cmd(store: &dyn store::Store, target: ExportTarget) -> Result<()> {
    let ExportTarget::Session { id, format, output } = target;
    let session = store
        .get_chat_session(&id)
        .await?
        .ok_or_else(|| CoworkerError::Workflow(format!("unknown chat session {id}")))?;
    let messages = store
        .list_active_branch_messages(&session, usize::MAX)
        .await?;
    let rendered = match format {
        ExportFormat::Jsonl => export_session_jsonl(&session, &messages),
        ExportFormat::Html => export_session_html(&session, &messages),
    };
    match output {
        Some(path) => {
            std::fs::write(&path, rendered)
                .map_err(|e| CoworkerError::Workflow(format!("write {}: {e}", path.display())))?;
            eprintln!("exported session {id} -> {}", path.display());
        }
        None => {
            print!("{rendered}");
        }
    }
    Ok(())
}

fn export_session_jsonl(
    session: &store::model::ChatSession,
    messages: &[store::model::ChatMessage],
) -> String {
    let mut out = String::new();
    let meta = serde_json::json!({
        "type": "session",
        "id": session.id.to_string(),
        "title": session.title,
        "created_at": session.created_at.to_rfc3339(),
        "repo_scope": session.repo_scope,
        "active_leaf_message_id": session.active_leaf_message_id.map(|u| u.to_string()),
    });
    out.push_str(&serde_json::to_string(&meta).unwrap());
    out.push('\n');
    for m in messages {
        let line = serde_json::json!({
            "type": "message",
            "id": m.id.to_string(),
            "parent_message_id": m.parent_message_id.map(|u| u.to_string()),
            "branch_index": m.branch_index,
            "role": serde_json::to_value(m.role).unwrap(),
            "content": m.content,
            "ts": m.ts.to_rfc3339(),
            "tool_name": m.tool_name,
            "tool_calls_json": m.tool_calls_json,
            "reasoning_original": m.reasoning_original,
        });
        out.push_str(&serde_json::to_string(&line).unwrap());
        out.push('\n');
    }
    out
}

fn export_session_html(
    session: &store::model::ChatSession,
    messages: &[store::model::ChatMessage],
) -> String {
    let mut body = String::new();
    for m in messages {
        let role = match m.role {
            store::model::ChatRole::User => "user",
            store::model::ChatRole::Assistant => "assistant",
            store::model::ChatRole::Tool => "tool",
            store::model::ChatRole::Harness => "harness",
            store::model::ChatRole::Reasoning => "reasoning",
        };
        let content = coworker_core::agent::redact::redact_json_str(&m.content);
        let label = match &m.tool_name {
            Some(t) => format!("{role}: {t}"),
            None => role.to_string(),
        };
        body.push_str(&format!(
            "<div class=\"msg {role}\"><div class=\"role\">{label}</div><pre>{escaped}</pre></div>\n",
            role = role,
            label = html_escape(&label),
            escaped = html_escape(&content),
        ));
    }
    let title = html_escape(&session.title);
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{title}</title>\
<style>body{{font-family:system-ui,sans-serif;margin:2rem;line-height:1.5}}\
.msg{{border:1px solid #ddd;border-radius:8px;padding:.75rem;margin:.75rem 0}}\
.role{{font-weight:600;font-size:.8rem;text-transform:uppercase;color:#666;margin-bottom:.25rem}}\
pre{{white-space:pre-wrap;word-break:break-word;margin:0}}\
.user{{background:#f0f7ff}}.assistant{{background:#f0fff4}}.tool{{background:#fff7f0}}\
.harness{{background:#fafafa}}.reasoning{{background:#fdf6ff}}</style>\
</head><body><h1>{title}</h1>{body}</body></html>\n",
        title = title,
        body = body,
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
