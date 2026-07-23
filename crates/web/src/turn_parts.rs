//! Build structured turn process parts for the Web UI.
//! - `chat_turn_parts`: in-flight turn only (`null` when idle)
//! - `chat_history_turn_parts`: per-user-line process parts (idle + busy)
//!
//! Shape mirrors `web-ui/src/tabs/chat/messageParts.ts`.

use serde_json::{json, Map, Value};
use std::collections::HashMap;

use coworker_core::app::AppState;

/// When `chat_busy`, emit process parts for the current agent turn.
pub fn build_chat_turn_parts(s: &AppState) -> Option<Vec<Value>> {
    if !s.chat_busy {
        return None;
    }
    let start = last_user_line_index(&s.chat_lines);
    Some(build_process_parts_for_range(s, start, s.chat_lines.len()))
}

/// Process parts keyed by the `you>` line index that starts each turn.
/// Emitted even when idle so history can render thinking/tools without
/// re-deriving from the line parser alone.
pub fn build_history_turn_parts(s: &AppState) -> Map<String, Value> {
    let lines = &s.chat_lines;
    let mut user_indices: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("you> ") {
            user_indices.push(i);
        }
    }

    let mut out = Map::new();
    for (ui, &user_idx) in user_indices.iter().enumerate() {
        let start = user_idx + 1;
        let end = user_indices.get(ui + 1).copied().unwrap_or(lines.len());
        // While busy, skip the in-flight turn — LiveZone owns `chat_turn_parts`.
        if s.chat_busy && ui + 1 == user_indices.len() {
            continue;
        }
        let parts = build_process_parts_for_range(s, start, end);
        if !parts.is_empty() {
            out.insert(user_idx.to_string(), Value::Array(parts));
        }
    }
    out
}

fn build_process_parts_for_range(s: &AppState, start: usize, end: usize) -> Vec<Value> {
    let lines = &s.chat_lines;
    if start >= end || start >= lines.len() {
        return vec![];
    }
    let end = end.min(lines.len());
    let slice = &lines[start..end];
    if slice.is_empty() {
        return vec![];
    }

    let outputs: HashMap<usize, &str> = s
        .chat_tool_outputs
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .collect();
    let tool_args: HashMap<usize, &str> = s
        .chat_tool_args
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .collect();
    let originals: HashMap<usize, &str> = s
        .chat_reasoning_originals
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .collect();

    let mut parts: Vec<Value> = Vec::new();
    let mut i = 0usize;
    while i < slice.len() {
        let abs = start + i;
        let line = slice[i].as_str();
        if line.starts_with("you> ") || line.starts_with("error> ") {
            i += 1;
            continue;
        }
        if line.starts_with("assistant> ") {
            if is_interim_assistant(slice, i) {
                parts.push(text_part(
                    abs,
                    "assistant",
                    line.strip_prefix("assistant> ").unwrap_or(""),
                ));
                i += 1;
                continue;
            }
            break;
        }
        if line.starts_with("chat> ") || line.starts_with("system> ") {
            i += 1;
            continue;
        }

        let mut steps: Vec<ToolStepLine> = Vec::new();
        while i < slice.len() {
            let l = slice[i].as_str();
            if l.starts_with("you> ") || l.starts_with("error> ") {
                break;
            }
            if l.starts_with("assistant> ") {
                if is_interim_assistant(slice, i) {
                    steps.push(ToolStepLine::interim(start + i, l));
                    i += 1;
                    continue;
                }
                break;
            }
            if l.starts_with("chat> ") || l.starts_with("system> ") {
                break;
            }
            steps.push(parse_tool_line(start + i, l, &outputs, &originals));
            i += 1;
        }
        push_step_parts(&mut parts, steps, &tool_args);
    }

    parts
}

fn last_user_line_index(lines: &[String]) -> usize {
    for (idx, line) in lines.iter().enumerate().rev() {
        if line.starts_with("you> ") {
            return idx + 1;
        }
    }
    0
}

fn is_interim_assistant(slice: &[String], idx: usize) -> bool {
    for l in slice.iter().skip(idx + 1) {
        let l = l.as_str();
        if l.trim().is_empty() {
            continue;
        }
        if l.starts_with("you> ") || l.starts_with("error> ") {
            return false;
        }
        if l.starts_with("assistant> ") {
            return false;
        }
        // Followed by tool/reasoning activity → interim prose inside the process.
        return l.starts_with("  ");
    }
    // Nothing after → final answer (not process).
    false
}

#[derive(Clone)]
struct ToolStepLine {
    index: usize,
    kind: StepKind,
    text: String,
    name: Option<String>,
    args: Option<String>,
    ms: Option<String>,
    ok: Option<bool>,
    #[allow(dead_code)]
    output: Option<String>,
    original: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StepKind {
    Reasoning,
    Start,
    Done,
    Warn,
    Interim,
    Meta,
}

impl ToolStepLine {
    fn interim(index: usize, line: &str) -> Self {
        Self {
            index,
            kind: StepKind::Interim,
            text: line.strip_prefix("assistant> ").unwrap_or(line).to_string(),
            name: None,
            args: None,
            ms: None,
            ok: None,
            output: None,
            original: None,
        }
    }
}

fn parse_tool_line(
    index: usize,
    line: &str,
    outputs: &HashMap<usize, &str>,
    originals: &HashMap<usize, &str>,
) -> ToolStepLine {
    let output = outputs.get(&index).copied().map(str::to_string);
    let original = originals.get(&index).copied().map(str::to_string);

    if line.starts_with("  … ") {
        let body = line.strip_prefix("  … ").unwrap_or(line);
        let text = if let Some(o) = output.as_deref() {
            o.to_string()
        } else {
            body.strip_prefix("reasoning: ")
                .or_else(|| body.strip_prefix("thinking"))
                .unwrap_or(body)
                .to_string()
        };
        return ToolStepLine {
            index,
            kind: StepKind::Reasoning,
            text,
            name: None,
            args: None,
            ms: None,
            ok: None,
            output,
            original,
        };
    }

    if line.starts_with("  → ") {
        let body = line.strip_prefix("  → ").unwrap_or(line);
        let (name, args) = split_tool_call(body);
        return ToolStepLine {
            index,
            kind: StepKind::Start,
            text: body.to_string(),
            name: Some(name),
            args,
            ms: None,
            ok: None,
            output,
            original: None,
        };
    }

    if line.starts_with("  ✓ ") || line.starts_with("  ✗ ") {
        let ok = line.starts_with("  ✓ ");
        let body = line
            .strip_prefix("  ✓ ")
            .or_else(|| line.strip_prefix("  ✗ "))
            .unwrap_or(line);
        let (name, args, ms) = split_tool_done(body);
        return ToolStepLine {
            index,
            kind: StepKind::Done,
            text: body.to_string(),
            name: Some(name),
            args,
            ms,
            ok: Some(ok),
            output,
            original: None,
        };
    }

    if line.starts_with("  ⚠ ") {
        return ToolStepLine {
            index,
            kind: StepKind::Warn,
            text: line.strip_prefix("  ⚠ ").unwrap_or(line).to_string(),
            name: None,
            args: None,
            ms: None,
            ok: None,
            output,
            original: None,
        };
    }

    ToolStepLine {
        index,
        kind: StepKind::Meta,
        text: line.to_string(),
        name: None,
        args: None,
        ms: None,
        ok: None,
        output,
        original: None,
    }
}

fn split_tool_call(body: &str) -> (String, Option<String>) {
    if let Some(open) = body.find('(') {
        if body.ends_with(')') {
            let name = body[..open].trim().to_string();
            let args = body[open + 1..body.len() - 1].trim().to_string();
            return (name, if args.is_empty() { None } else { Some(args) });
        }
    }
    (body.trim().to_string(), None)
}

fn split_tool_done(body: &str) -> (String, Option<String>, Option<String>) {
    let ms = body.rsplit_once('(').and_then(|(rest, tail)| {
        tail.strip_suffix("ms)")?;
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            None
        } else {
            Some((rest.trim_end(), digits))
        }
    });
    let (rest, ms) = match ms {
        Some((r, m)) => (r, Some(m)),
        None => (body, None),
    };
    let (name, args) = split_tool_call(rest);
    (name, args, ms)
}

fn push_step_parts(
    parts: &mut Vec<Value>,
    steps: Vec<ToolStepLine>,
    tool_args: &HashMap<usize, &str>,
) {
    if steps.is_empty() {
        return;
    }
    if steps.iter().all(|s| s.kind == StepKind::Reasoning) {
        let text = steps
            .iter()
            .map(|s| s.text.as_str())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        if !text.is_empty() {
            let first = &steps[0];
            parts.push(reasoning_part(
                first.index,
                &text,
                first.original.as_deref(),
            ));
        }
        return;
    }

    let mut pending: Vec<Vec<ToolStepLine>> = Vec::new();
    for step in steps {
        match step.kind {
            StepKind::Start => pending.push(vec![step]),
            StepKind::Done => {
                let name = step.name.clone();
                if let Some(pos) = pending.iter().rposition(|group| {
                    group
                        .iter()
                        .find(|s| s.kind == StepKind::Start)
                        .and_then(|s| s.name.as_ref())
                        == name.as_ref()
                }) {
                    pending[pos].push(step.clone());
                    let finished = pending.remove(pos);
                    parts.push(tool_part_from_group(&finished, tool_args));
                } else {
                    parts.push(tool_part_from_group(&[step], tool_args));
                }
            }
            StepKind::Interim => {
                parts.push(text_part(step.index, "assistant", &step.text));
            }
            StepKind::Reasoning => {
                parts.push(reasoning_part(
                    step.index,
                    &step.text,
                    step.original.as_deref(),
                ));
            }
            StepKind::Warn | StepKind::Meta => {
                if let Some(last) = pending.last_mut() {
                    last.push(step);
                } else {
                    parts.push(tool_part_from_group(&[step], tool_args));
                }
            }
        }
    }
    for group in pending {
        parts.push(tool_part_from_group(&group, tool_args));
    }
}

fn reasoning_part(index: usize, text: &str, original: Option<&str>) -> Value {
    let mut v = json!({
        "id": format!("reasoning-{index}"),
        "kind": "reasoning",
        "text": text,
    });
    if let Some(o) = original.filter(|o| !o.is_empty()) {
        v["original"] = json!(o);
    }
    v
}

fn text_part(index: usize, role: &str, text: &str) -> Value {
    json!({
        "id": format!("text-{index}"),
        "kind": "text",
        "role": role,
        "text": text,
        "md": true,
    })
}

fn tool_part_from_group(group: &[ToolStepLine], tool_args: &HashMap<usize, &str>) -> Value {
    let index = group.first().map(|s| s.index).unwrap_or(0);
    let start = group.iter().find(|s| s.kind == StepKind::Start);
    let done = group.iter().find(|s| s.kind == StepKind::Done);
    let tool_name = done
        .and_then(|s| s.name.clone())
        .or_else(|| start.and_then(|s| s.name.clone()))
        .unwrap_or_else(|| "tool".into());
    let status = if done.is_some() {
        if done.and_then(|s| s.ok).unwrap_or(true) {
            "ok"
        } else {
            "err"
        }
    } else if start.is_some() {
        "running"
    } else {
        "neutral"
    };
    let ms = done.and_then(|s| s.ms.clone());
    let args = start
        .and_then(|s| tool_args.get(&s.index).copied())
        .map(pretty_tool_args_json)
        .or_else(|| {
            done.and_then(|s| s.args.clone())
                .or_else(|| start.and_then(|s| s.args.clone()))
        });
    let block_key = format!("tool-{index}-{tool_name}");
    let steps: Vec<Value> = group
        .iter()
        .filter(|s| s.kind != StepKind::Meta)
        .map(tool_step_json)
        .collect();
    json!({
        "id": block_key,
        "kind": "tool",
        "blockKey": block_key,
        "group": {
            "toolName": tool_name,
            "status": status,
            "ms": ms,
            "args": args,
            "steps": steps,
        }
    })
}

fn tool_step_json(step: &ToolStepLine) -> Value {
    let kind = match step.kind {
        StepKind::Start => "start",
        StepKind::Done => "done",
        StepKind::Warn => "warn",
        StepKind::Reasoning => "reasoning",
        StepKind::Interim => "interim",
        StepKind::Meta => "meta",
    };
    let mut v = json!({
        "kind": kind,
        "text": step.text,
        "index": step.index,
    });
    if let Some(name) = &step.name {
        v["name"] = json!(name);
    }
    if let Some(args) = &step.args {
        v["args"] = json!(args);
    }
    if let Some(ok) = step.ok {
        v["ok"] = json!(ok);
    }
    if let Some(ms) = &step.ms {
        v["ms"] = json!(ms);
    }
    if let Some(output) = &step.output {
        v["output"] = json!(output);
    }
    if let Some(original) = &step.original {
        v["original"] = json!(original);
    }
    v
}

fn pretty_tool_args_json(raw: &str) -> String {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use coworker_core::app::AppState;
    use coworker_core::config::Config;

    fn test_app() -> AppState {
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
chat: { enabled: true }
storage: { backend: json, path: ./data }
"#;
        AppState::new(Config::load_from_str(yaml).unwrap(), "coworker.yaml".into())
    }

    #[test]
    fn turn_parts_none_when_idle() {
        let app = test_app();
        assert!(build_chat_turn_parts(&app).is_none());
    }

    #[test]
    fn turn_parts_emits_reasoning_and_tool() {
        let mut app = test_app();
        app.chat_busy = true;
        app.push_chat_line("you> hi");
        app.push_chat_line("  … reasoning: thinking…");
        app.record_chat_tool_output(1, "full reasoning body".into());
        app.push_chat_line("  → grep(pattern=foo)");
        app.push_chat_line("  ✓ grep(pattern=foo)(12ms)");
        app.record_chat_tool_output(3, "match at line 1".into());
        let parts = build_chat_turn_parts(&app).expect("parts");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["kind"], "reasoning");
        assert_eq!(parts[1]["kind"], "tool");
        assert_eq!(parts[1]["group"]["toolName"], "grep");
        assert_eq!(parts[1]["group"]["status"], "ok");
        let steps = parts[1]["group"]["steps"].as_array().expect("steps");
        assert!(steps.len() >= 2);
        assert_eq!(steps.last().unwrap()["output"], "match at line 1");
    }

    #[test]
    fn history_turn_parts_when_idle() {
        let mut app = test_app();
        app.chat_busy = false;
        app.push_chat_line("you> first");
        app.push_chat_line("  … reasoning: think");
        app.record_chat_tool_output(1, "thought body".into());
        app.push_chat_line("  → read_file(path=a.ts)");
        app.push_chat_line("  ✓ read_file(path=a.ts)(5ms)");
        app.record_chat_tool_output(3, "export const a".into());
        app.push_chat_line("assistant> done");
        app.push_chat_line("you> second");
        app.push_chat_line("assistant> ok");

        let hist = build_history_turn_parts(&app);
        let first = hist.get("0").expect("turn 0").as_array().expect("arr");
        assert!(first.iter().any(|p| p["kind"] == "reasoning"));
        assert!(first.iter().any(|p| p["kind"] == "tool"));
        assert!(hist.get("5").is_none()); // answer-only turn has no process
    }

    #[test]
    fn history_skips_in_flight_turn_while_busy() {
        let mut app = test_app();
        app.chat_busy = true;
        app.push_chat_line("you> done");
        app.push_chat_line("  → grep(pattern=old)");
        app.push_chat_line("  ✓ grep(pattern=old)(1ms)");
        app.push_chat_line("assistant> answer");
        app.push_chat_line("you> live");
        app.push_chat_line("  → grep(pattern=x)");
        let hist = build_history_turn_parts(&app);
        let completed = hist.get("0").expect("completed turn").as_array().unwrap();
        assert!(completed.iter().any(|p| p["kind"] == "tool"));
        assert!(hist.get("4").is_none()); // in-flight skipped
        let live = build_chat_turn_parts(&app).expect("live");
        assert_eq!(live[0]["kind"], "tool");
    }
}
