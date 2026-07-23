use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio::sync::Mutex;
use tokio::time::{self, MissedTickBehavior};

use crate::agent::ask_user_tool::{self, AskUserPause};
use crate::agent::bash_tool;
use crate::agent::chat_discovery::ChatDiscoveryState;
use crate::agent::context::format_tool_context_message;
use crate::agent::file_edit_tool;
use crate::agent::file_tools;
use crate::agent::harness_errors::agent_validation_error;
use crate::agent::harness_tools;
use crate::agent::python_tool;
use crate::agent::review_gate::ReviewGateOutcome;
use crate::agent::web_fetch_tool;
use crate::config::{BashToolConfig, Config, PythonToolConfig};
use crate::engine::SkillRegistry;
use crate::error::{CoworkerError, Result};
use crate::github::helpers::{gh_tool, gh_tool_with_retry, read_resource};
use crate::github::GithubHarness;
use crate::llm::LlmClient;
use crate::llm::LlmTurnMessage;
use crate::mcp::McpPool;
use crate::store::{ChatRole, Store};
use uuid::Uuid;

use crate::agent::chat_duplicate::{
    maybe_push_tool_failure_harness_nudge, prune_stale_missing_arg_nudges,
    remove_satisfied_missing_arg_nudges,
};
use crate::agent::chat_loop::{
    activity_flow_kind_for_tool, append_message, append_tool_result_message, ci_analyze_lacks_runs,
    emit_activity_flow, emit_activity_flow_clear, emit_progress, ensure_chat_not_cancelled,
    finalize_tool_args, format_flow_tool_done, format_flow_tool_start, format_tool_args_short,
    is_flow_activity_tool, is_mutating_tool, race_chat_cancel, tool_output_indicates_failure,
    ChatProgress, PreparedToolCall, ToolCallSummary, ToolExecRecord, ToolRoundState,
};

#[derive(Debug, Clone)]
pub(crate) struct ReadonlyToolOutcome {
    pub(crate) call: PreparedToolCall,
    pub(crate) output: String,
    pub(crate) ok: bool,
    pub(crate) llm_review_rejected: Option<crate::agent::bash_tool::BashCommandReview>,
    pub(crate) awaiting_user: Option<AskUserPause>,
}

pub(crate) struct ReadonlyToolHarness {
    pub(crate) store: Arc<dyn Store>,
    pub(crate) github: Arc<GithubHarness>,
    pub(crate) mcp: Arc<McpPool>,
    pub(crate) discovery: Arc<Mutex<ChatDiscoveryState>>,
    pub(crate) cancel: Option<Arc<AtomicBool>>,
    pub(crate) progress: Option<broadcast::Sender<crate::app::AppEvent>>,
}

pub(crate) struct ReadonlyToolContext<'a> {
    pub(crate) user_task: &'a str,
    pub(crate) bash: &'a BashToolConfig,
    pub(crate) python: &'a PythonToolConfig,
    pub(crate) workspace: &'a std::path::Path,
    pub(crate) llm: Arc<LlmClient>,
    pub(crate) config: Arc<Config>,
    pub(crate) progress: Option<broadcast::Sender<crate::app::AppEvent>>,
    pub(crate) cancel: Option<Arc<AtomicBool>>,
}

pub(crate) async fn execute_readonly_tools_parallel(
    harness: ReadonlyToolHarness,
    ctx: ReadonlyToolContext<'_>,
    calls: Vec<PreparedToolCall>,
) -> Result<Vec<ReadonlyToolOutcome>> {
    let futures = calls.into_iter().map(|call| {
        let harness = ReadonlyToolHarness {
            store: Arc::clone(&harness.store),
            github: Arc::clone(&harness.github),
            mcp: Arc::clone(&harness.mcp),
            discovery: Arc::clone(&harness.discovery),
            cancel: harness.cancel.clone(),
            progress: harness.progress.clone(),
        };
        let user_task = ctx.user_task.to_string();
        let bash = ctx.bash.clone();
        let python = ctx.python.clone();
        let workspace = ctx.workspace.to_path_buf();
        let llm = Arc::clone(&ctx.llm);
        let config = Arc::clone(&ctx.config);
        let progress_ctx = ctx.progress.clone();
        let discovery = Arc::clone(&harness.discovery);
        let progress_tx = harness.progress.clone();
        let cancel_flag = harness.cancel.clone();
        async move {
            ensure_chat_not_cancelled(&cancel_flag)?;
            let args_short = format_tool_args_short(&call.args);
            let flow_tool = is_flow_activity_tool(&call.name);
            if flow_tool {
                emit_activity_flow(
                    &progress_tx,
                    activity_flow_kind_for_tool(&call.name),
                    format_flow_tool_start(&call.name, &call.args),
                );
            } else {
                emit_progress(
                    &progress_tx,
                    ChatProgress::ToolStart {
                        name: call.name.clone(),
                        args_short,
                        tool_args_json: serde_json::to_string(&call.args).unwrap_or_default(),
                    },
                );
            }
            let tool_start = Instant::now();
            let result = match race_chat_cancel(
                cancel_flag.clone(),
                execute_readonly_tool_with_heartbeat(
                    harness,
                    &discovery,
                    ReadonlyToolContext {
                        user_task: &user_task,
                        bash: &bash,
                        python: &python,
                        workspace: &workspace,
                        llm,
                        config,
                        progress: progress_ctx,
                        cancel: cancel_flag,
                    },
                    &call.name,
                    call.args.clone(),
                ),
            )
            .await
            {
                Ok(r) => r,
                Err(e) => return Err(e),
            };
            let (output, ok, llm_review_rejected, awaiting_user) = match result {
                Ok(ReadonlyToolExecuteResult::Output(o))
                    if tool_output_indicates_failure(&call.name, &o) =>
                {
                    (o, false, None, None)
                }
                Ok(ReadonlyToolExecuteResult::Output(o)) => (o, true, None, None),
                Ok(ReadonlyToolExecuteResult::LlmReviewRejected(review)) => {
                    (String::new(), false, Some(review), None)
                }
                Ok(ReadonlyToolExecuteResult::AwaitingUser(mut pause)) => {
                    pause.tool_call_id = call.id.clone();
                    (String::new(), true, None, Some(pause))
                }
                Err(e) => (format!("tool error: {e}"), false, None, None),
            };
            let elapsed_ms = tool_start.elapsed().as_millis();
            if awaiting_user.is_some() {
                return Ok(ReadonlyToolOutcome {
                    call,
                    output,
                    ok,
                    llm_review_rejected,
                    awaiting_user,
                });
            }
            let ctx = format_tool_context_message(&call.name, &call.args, ok, &output);
            if flow_tool {
                emit_activity_flow(
                    &progress_tx,
                    activity_flow_kind_for_tool(&call.name),
                    format_flow_tool_done(&call.name, &call.args, ok, &ctx),
                );
                emit_activity_flow_clear(&progress_tx);
            } else {
                emit_progress(
                    &progress_tx,
                    ChatProgress::ToolDone {
                        name: call.name.clone(),
                        args_short: format_tool_args_short(&call.args),
                        ok,
                        elapsed_ms,
                        output_preview: crate::agent::context::truncate_chars(&ctx, 6_000),
                    },
                );
            }
            Ok(ReadonlyToolOutcome {
                call,
                output,
                ok,
                llm_review_rejected,
                awaiting_user: None,
            })
        }
    });
    join_all(futures).await.into_iter().collect()
}

pub(crate) async fn record_tool_outcome(
    round: &mut ToolRoundState<'_>,
    outcome: ReadonlyToolOutcome,
    mcp: Option<&std::sync::Arc<crate::mcp::McpPool>>,
) -> Result<()> {
    let PreparedToolCall {
        id,
        name,
        args,
        fingerprint,
    } = outcome.call;
    let output = outcome.output;
    let ok = outcome.ok;
    let ctx = format_tool_context_message(&name, &args, ok, &output);
    round.tool_calls.push(ToolCallSummary {
        tool_name: name.clone(),
        output: ctx.clone(),
    });
    let record = round
        .tool_exec_records
        .entry(fingerprint.clone())
        .or_insert(ToolExecRecord {
            succeeded: false,
            fail_count: 0,
        });
    if ok {
        record.succeeded = true;
        round.duplicate_tool_nudges.remove(&fingerprint);
        round.duplicate_ui_shown.remove(&fingerprint);
        let mut state = round.discovery.lock().await;
        state.warm_from_tool_call_args(&name, &args);
        if let Some(mcp) = mcp {
            let warm_name = if name == "tool_call" {
                args.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(name.as_str())
            } else {
                name.as_str()
            };
            state.warm_tool_from_registry(warm_name, mcp).await;
        }
    } else {
        record.fail_count += 1;
    }
    append_tool_result_message(
        round.store,
        round.session_id,
        &ctx,
        &name,
        args.to_string(),
        Some(id.as_str()),
    )
    .await?;
    round.llm_messages.push(LlmTurnMessage::tool_result_with_id(
        Some(id),
        name.clone(),
        ctx.clone(),
    ));
    if ok {
        remove_satisfied_missing_arg_nudges(round.llm_messages, &name, &args);
        prune_stale_missing_arg_nudges(round.llm_messages);
    } else {
        let nudge = maybe_push_tool_failure_harness_nudge(
            round.tool_catalog,
            &name,
            &args,
            &output,
            round.llm_messages,
        );
        append_message(
            round.store,
            round.session_id,
            ChatRole::Harness,
            &nudge,
            None,
            None,
            None,
        )
        .await?;
    }
    if ok && name == "ci_analyze_pr_failures" && ci_analyze_lacks_runs(&output) {
        round.llm_messages.push(LlmTurnMessage::new(
            "user",
            "ci_analyze returned no actionable run IDs in this response \
(pending checks or empty output).",
        ));
    }
    Ok(())
}

enum ReadonlyToolExecuteResult {
    Output(String),
    LlmReviewRejected(crate::agent::bash_tool::BashCommandReview),
    AwaitingUser(AskUserPause),
}

fn wrap_review_gate(outcome: ReviewGateOutcome) -> ReadonlyToolExecuteResult {
    match outcome {
        ReviewGateOutcome::Executed(s) => ReadonlyToolExecuteResult::Output(s),
        ReviewGateOutcome::LlmRejected(r) => ReadonlyToolExecuteResult::LlmReviewRejected(r),
    }
}

async fn execute_readonly_tool_with_heartbeat(
    harness: ReadonlyToolHarness,
    discovery: &Arc<Mutex<ChatDiscoveryState>>,
    ctx: ReadonlyToolContext<'_>,
    tool_name: &str,
    tool_args: Value,
) -> Result<ReadonlyToolExecuteResult> {
    let name = tool_name.to_string();
    let args = tool_args.clone();
    let progress = harness.progress.clone();
    let discovery = Arc::clone(discovery);
    let mut tool_fut = Box::pin(execute_readonly_tool(
        harness.store,
        harness.github,
        harness.mcp,
        &discovery,
        ctx,
        tool_name,
        tool_args,
    ));
    let started = Instant::now();
    let mut tick = time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tick.tick().await;

    loop {
        tokio::select! {
            result = &mut tool_fut => return result,
            _ = tick.tick() => {
                let detail = format_tool_progress_detail(tool_name, &args, started.elapsed());
                emit_progress(
                    &progress,
                    ChatProgress::ToolProgress {
                        name: name.clone(),
                        detail,
                    },
                );
            }
        }
    }
}

/// Elapsed / paging hint for the TUI while a readonly tool is in flight.
pub(crate) fn format_tool_progress_detail(
    tool_name: &str,
    args: &Value,
    elapsed: Duration,
) -> String {
    let secs = elapsed.as_secs();
    match tool_name {
        "ci_get_failed_logs" => {
            let offset = args
                .get("offset_lines")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let max_lines = args.get("max_lines").and_then(|v| v.as_u64()).unwrap_or(0);
            if max_lines > 0 {
                let page = offset.checked_div(max_lines).unwrap_or(0).saturating_add(1);
                format!("page {page}, {secs}s")
            } else {
                format!("fetching logs, {secs}s")
            }
        }
        "ci_get_run_summary" | "ci_analyze_pr_failures" | "pr_get_overview" | "pr_get_diff" => {
            format!("{secs}s")
        }
        "bash_run" => {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{cmd}, {secs}s")
        }
        "python_run" => {
            let lines = args
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .lines()
                .next()
                .unwrap_or("?");
            format!("{lines}, {secs}s")
        }
        "web_fetch" | "web_browser" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{url}, {secs}s")
        }
        _ => format!("{secs}s"),
    }
}

async fn execute_readonly_tool(
    store: Arc<dyn Store>,
    github: Arc<GithubHarness>,
    mcp: Arc<McpPool>,
    discovery: &Arc<Mutex<ChatDiscoveryState>>,
    ctx: ReadonlyToolContext<'_>,
    tool_name: &str,
    mut tool_args: Value,
) -> Result<ReadonlyToolExecuteResult> {
    finalize_tool_args(tool_name, &mut tool_args, ctx.user_task);
    if ask_user_tool::is_ask_user_tool(tool_name) {
        let request = ask_user_tool::parse_ask_user_args(&tool_args)?;
        return Ok(ReadonlyToolExecuteResult::AwaitingUser(AskUserPause {
            question_id: Uuid::new_v4(),
            request,
            tool_call_id: String::new(),
            tool_args,
        }));
    }
    if harness_tools::is_harness_tool(tool_name) {
        return Ok(ReadonlyToolExecuteResult::Output(
            harness_tools::execute_harness_tool(store.as_ref(), tool_name, tool_args).await?,
        ));
    }
    if bash_tool::is_bash_tool(tool_name) {
        return Ok(wrap_review_gate(
            bash_tool::execute_bash_tool(ctx.bash, ctx.llm.as_ref(), ctx.workspace, &tool_args)
                .await?,
        ));
    }
    if python_tool::is_python_tool(tool_name) {
        return Ok(wrap_review_gate(
            python_tool::execute_python_tool(
                ctx.python,
                ctx.llm.as_ref(),
                ctx.workspace,
                &tool_args,
            )
            .await?,
        ));
    }
    if file_tools::is_mutating_file_tool(tool_name) {
        return Ok(wrap_review_gate(
            file_edit_tool::execute_mutating_file_tool_with_review(
                ctx.workspace,
                ctx.llm.as_ref(),
                tool_name,
                &tool_args,
            )
            .await?,
        ));
    }
    if web_fetch_tool::is_web_fetch_tool(tool_name) {
        return Ok(ReadonlyToolExecuteResult::Output(
            web_fetch_tool::execute_web_fetch_tool(
                &ctx.config.chat.web_fetch,
                ctx.workspace,
                &tool_args,
            )
            .await?,
        ));
    }
    if file_tools::is_file_tool(tool_name) {
        return Ok(ReadonlyToolExecuteResult::Output(
            file_tools::execute_file_tool(ctx.workspace, tool_name, &tool_args)?,
        ));
    }
    if tool_name == "tool_list" {
        if let Some(cached) =
            crate::agent::hooks::tool_list_cached_response(&*discovery.lock().await)
        {
            return Ok(ReadonlyToolExecuteResult::Output(cached));
        }
        let text = if mcp.has_servers() {
            crate::mcp::federated_tool_list(mcp.as_ref()).await
        } else {
            gh_tool(github.as_ref(), "tool_list", json!({})).await?
        };
        discovery.lock().await.store_tool_list(text.clone());
        return Ok(ReadonlyToolExecuteResult::Output(text));
    }
    if tool_name == "tool_search" {
        let query = tool_args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                agent_validation_error(
                    "tool_search",
                    "TOOL_MISSING_ARG",
                    "tool_search needs query",
                    "Pass a short tool name keyword",
                )
            })?;
        let limit = tool_args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
        let text = if mcp.has_servers() {
            crate::mcp::federated_tool_search(mcp.as_ref(), query, limit).await?
        } else {
            let mut args = json!({ "query": query });
            if let Some(limit) = tool_args.get("limit") {
                args["limit"] = limit.clone();
            }
            gh_tool(github.as_ref(), "tool_search", args).await?
        };
        return Ok(ReadonlyToolExecuteResult::Output(text));
    }
    if tool_name == "tool_describe" {
        let name = tool_args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                agent_validation_error(
                    "tool_describe",
                    "TOOL_MISSING_ARG",
                    "tool_describe needs name",
                    "Pass exact tool name from tool_search",
                )
            })?;
        let text = if mcp.has_servers() {
            crate::mcp::federated_tool_describe(mcp.as_ref(), name).await?
        } else {
            gh_tool(github.as_ref(), "tool_describe", json!({ "name": name })).await?
        };
        return Ok(ReadonlyToolExecuteResult::Output(text));
    }
    if mcp.is_mcp_tool_async(tool_name).await {
        return Ok(ReadonlyToolExecuteResult::Output(
            mcp.call_global_tool(tool_name, tool_args, ctx.cancel.clone())
                .await?,
        ));
    }
    Ok(ReadonlyToolExecuteResult::Output(match tool_name {
        "tool_list_category" => {
            let category = tool_args
                .get("category")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    agent_validation_error(
                        "tool_list_category",
                        "TOOL_MISSING_ARG",
                        "tool_list_category needs category",
                        "Pass category from tool_list",
                    )
                })?;
            gh_tool(
                github.as_ref(),
                "tool_list_category",
                json!({ "category": category }),
            )
            .await?
        }
        "resource_read" => {
            let uri = tool_args
                .get("uri")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    agent_validation_error(
                        "resource_read",
                        "TOOL_MISSING_ARG",
                        "resource_read needs uri",
                        "Use github://, pr://, or mcp+{server}:// URI from tool_describe",
                    )
                })?;
            if uri.starts_with("mcp+") {
                mcp.read_federated_resource(uri, ctx.cancel.clone()).await?
            } else {
                read_resource(github.as_ref(), uri).await?
            }
        }
        "skill_load" => {
            let name = tool_args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    agent_validation_error(
                        "skill_load",
                        "TOOL_MISSING_ARG",
                        "skill_load needs name",
                        "Pass skill name from **Available skills** in the system prompt",
                    )
                })?;
            let mut state = discovery.lock().await;
            let skill = state.skill_registry.get(name).cloned().ok_or_else(|| {
                agent_validation_error(
                    "skill_load",
                    "TOOL_NOT_FOUND",
                    format!("unknown skill {name:?}"),
                    "Pick a name from **Available skills** in the system prompt",
                )
            })?;
            state.warm_skill_tools(&skill);
            for tool in &skill.tool_refs {
                state.warm_tool_from_registry(tool, mcp.as_ref()).await;
            }
            SkillRegistry::format_skill_load(&skill)
        }
        "tool_call" => {
            let name = tool_args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    agent_validation_error(
                        "tool_call",
                        "TOOL_MISSING_ARG",
                        "tool_call needs name",
                        "Pass { \"name\": \"...\", \"args\": { ... } }",
                    )
                })?;
            let args = tool_args.get("args").cloned().unwrap_or_else(|| json!({}));
            if is_mutating_tool(name) {
                return Err(CoworkerError::Workflow(format!(
                    "{name} is mutating — use approval action"
                )));
            }
            if mcp.is_mcp_tool_async(name).await {
                mcp.call_global_tool(name, args, ctx.cancel.clone()).await?
            } else {
                gh_tool(
                    github.as_ref(),
                    "tool_call",
                    json!({ "name": name, "args": args }),
                )
                .await?
            }
        }
        other if is_mutating_tool(other) => {
            return Err(CoworkerError::Workflow(format!(
                "{other} is mutating — use approval action"
            )));
        }
        other => gh_tool_with_retry(github.as_ref(), other, tool_args).await?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn format_tool_progress_detail_for_paged_logs() {
        let args = json!({"offset_lines": 160, "max_lines": 80});
        let detail =
            format_tool_progress_detail("ci_get_failed_logs", &args, Duration::from_secs(12));
        assert_eq!(detail, "page 3, 12s");

        let args = json!({});
        let detail =
            format_tool_progress_detail("ci_get_failed_logs", &args, Duration::from_secs(4));
        assert_eq!(detail, "fetching logs, 4s");
    }
}
