use std::future::Future;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use coworker_core::agent::chat_loop::{ChatProgress, ChatTurnResult, ResumeChatAfterApproval};
use rustyline::config::Configurer;
use rustyline::{ColorMode, DefaultEditor};

use coworker_core::app::{event_channel, AppEvent, AppState, SharedState};
use coworker_core::config::Config;
use coworker_core::engine::Engine;
use coworker_core::error::Result;
use coworker_core::store;

use coworker_core::exit_codes;

use super::terminal::{
    colorize_progress, emit_json, err_prefix, hint_prefix, reasoning_tail, render_markdown,
    spinner_frame, table, timeout_prefix, tool_block_done, tool_block_start, use_color_stdout,
    warn_prefix,
};

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_chat_cli(
    config: Config,
    store: Arc<dyn store::Store>,
    once: Option<String>,
    session: Option<uuid::Uuid>,
    json: bool,
    mut title: Option<String>,
    yes: bool,
    timeout: Option<u64>,
) -> Result<()> {
    if !config.chat.enabled {
        return Err(coworker_core::error::CoworkerError::Workflow(
            "chat disabled — set chat.enabled: true in coworker.yaml".into(),
        ));
    }

    let (tx, mut rx) = event_channel();
    let state: SharedState = Arc::new(tokio::sync::RwLock::new(AppState::new(
        config.clone(),
        "chat-cli".into(),
    )));
    let histpath = cli_history_path(&config);
    let engine = Arc::new(Engine::new(config, Arc::clone(&store), tx, Arc::clone(&state)).await);

    let mut session_id = session;

    // --once: single turn, script-friendly, with an optional approval loop.
    if let Some(msg) = once {
        let run_once = async {
            let (mut result, mut streamed, mut pending, _pending_q) = run_turn_with_progress(
                &engine,
                &mut rx,
                json,
                None,
                !json,
                engine.run_chat(session_id, &msg),
            )
            .await?;
            while result.awaiting_approval {
                let pa = match pending {
                    Some(p) => p,
                    None => break,
                };
                if !yes {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "ok": false,
                                "error": "awaiting approval",
                                "awaiting_approval": true,
                                "session_id": result.session_id,
                                "pending_approval": serde_json::json!({
                                    "tool": pa.tool_name,
                                    "args": coworker_core::agent::redact::redact_json_str(&pa.tool_args_json),
                                    "description": pa.description,
                                }),
                            })
                        );
                    } else {
                        eprintln!(
                            "{} for `{}` — {}",
                            warn_prefix().replace("warning:", "approval required"),
                            pa.tool_name,
                            pa.description
                        );
                        eprintln!(
                            "  {} re-run with --yes to auto-approve, or use interactive `chat` to approve per-tool.",
                            hint_prefix()
                        );
                    }
                    std::process::exit(exit_codes::EXIT_APPROVAL);
                }
                let detail = engine
                    .decide_approval(&pa.approval_id, true, None)
                    .await
                    .unwrap_or_else(|e| {
                        eprintln!("approval error: {e}");
                        e.to_string()
                    });
                let tool_args = serde_json::from_str(&pa.tool_args_json)
                    .unwrap_or_else(|_| serde_json::json!({}));
                let resume = ResumeChatAfterApproval {
                    approval_id: pa.approval_id,
                    approved: true,
                    detail,
                    tool_name: pa.tool_name.clone(),
                    tool_args,
                    tool_call_id: pa.tool_call_id.clone(),
                };
                let (r, s, p, _) = run_turn_with_progress(
                    &engine,
                    &mut rx,
                    json,
                    None,
                    !json,
                    engine.resume_chat_after_approval(pa.session_id, resume),
                )
                .await?;
                result = r;
                streamed = s;
                pending = p;
            }
            Ok::<_, coworker_core::error::CoworkerError>((result, streamed))
        };

        let turn_result = match timeout {
            Some(secs) => {
                match tokio::time::timeout(std::time::Duration::from_secs(secs), run_once).await {
                    Ok(r) => r,
                    Err(_) => {
                        if json {
                            emit_json(serde_json::json!({ "ok": false, "error": "timeout" }));
                        } else {
                            eprintln!("{} after {secs}s", timeout_prefix());
                            eprintln!(
                                "  {} increase --timeout or check LLM latency",
                                hint_prefix()
                            );
                        }
                        std::process::exit(exit_codes::EXIT_TIMEOUT);
                    }
                }
            }
            None => run_once.await,
        };

        match turn_result {
            Ok((result, streamed)) => {
                maybe_apply_title(&store, result.session_id, title.as_deref()).await;
                if json {
                    let tools: Vec<_> = result
                        .tool_calls
                        .iter()
                        .map(|tc| serde_json::json!({ "tool": tc.tool_name, "output": tc.output }))
                        .collect();
                    emit_json(serde_json::json!({
                        "ok": true,
                        "session_id": result.session_id,
                        "assistant": result.assistant_message,
                        "tool_calls": tools,
                        "awaiting_approval": result.awaiting_approval,
                        "awaiting_user_input": result.awaiting_user_input,
                    }));
                } else {
                    if !streamed {
                        println!("{}", result.assistant_message);
                    } else {
                        println!();
                    }
                }
                return Ok(());
            }
            Err(e) => {
                if json {
                    emit_json(serde_json::json!({ "ok": false, "error": e.to_string() }));
                } else {
                    eprintln!("{} {e}", err_prefix());
                }
                std::process::exit(exit_codes::EXIT_GENERAL);
            }
        }
    }

    // Interactive REPL — rustyline for line editing + persistent history.
    let mut rl = DefaultEditor::new().map_err(|e| {
        coworker_core::error::CoworkerError::Workflow(format!("rustyline init failed: {e}"))
    })?;
    let _ = rl.load_history(&histpath);
    if std::io::stdout().is_terminal() {
        rl.set_color_mode(ColorMode::Enabled);
    }
    let rl = Arc::new(std::sync::Mutex::new(rl));

    eprintln!("unistar-coworker chat — /help for commands, Ctrl-C cancels a turn, Ctrl-D to quit");

    let mut last_reply: Option<String> = None;

    loop {
        let prompt = repl_prompt(session_id);
        let rl2 = Arc::clone(&rl);
        let readline = tokio::task::spawn_blocking(move || {
            let mut g = rl2.lock().expect("rl mutex poisoned");
            g.readline(&prompt)
        })
        .await;
        let raw = match readline {
            Ok(Ok(line)) => line,
            Ok(Err(rustyline::error::ReadlineError::Interrupted)) => continue,
            Ok(Err(rustyline::error::ReadlineError::Eof)) => break,
            Ok(Err(_)) => break,
            Err(_) => break,
        };
        let text = raw.trim();
        if text.is_empty() {
            continue;
        }
        {
            let mut g = rl.lock().expect("rl mutex poisoned");
            let _ = g.add_history_entry(raw.as_str());
        }
        if text == "quit" || text == "exit" {
            break;
        }
        if let Some(stripped) = text.strip_prefix('/') {
            let mut parts = stripped.split_whitespace();
            let name = parts.next().unwrap_or("").to_string();
            let arg = parts.next().map(|s| s.to_string());
            match name.as_str() {
                "resume" | "r" => {
                    handle_resume(&store, &rl, &mut session_id, arg).await?;
                    last_reply = None;
                    continue;
                }
                "retry" => {
                    let sid = match session_id {
                        Some(id) => id,
                        None => {
                            eprintln!("(no session to retry — send a message first)");
                            continue;
                        }
                    };
                    let messages = match store.list_chat_messages(&sid, 200).await {
                        Ok(m) => m,
                        Err(e) => {
                            eprintln!("{} {e}", err_prefix());
                            continue;
                        }
                    };
                    let last_assistant = messages
                        .iter()
                        .rev()
                        .find(|m| m.role == store::model::ChatRole::Assistant)
                        .map(|m| m.id);
                    match last_assistant {
                        Some(aid) => {
                            eprintln!("(regenerating from assistant {aid})");
                            match run_repl_turn(&engine, &mut rx, &rl, Some(sid), "", Some(aid))
                                .await
                            {
                                Ok((s, reply)) => {
                                    session_id = Some(s);
                                    last_reply = Some(reply);
                                }
                                Err(e) => eprintln!("{} {e}\n", err_prefix()),
                            }
                        }
                        None => eprintln!("(no assistant message to regenerate)"),
                    }
                    continue;
                }
                "history" | "hist" => {
                    let sid = match session_id {
                        Some(id) => id,
                        None => {
                            eprintln!("(no active session)");
                            continue;
                        }
                    };
                    let limit = arg.and_then(|a| a.parse::<usize>().ok()).unwrap_or(50);
                    let msgs = match store.list_chat_messages(&sid, limit).await {
                        Ok(m) => m,
                        Err(e) => {
                            eprintln!("{} {e}", err_prefix());
                            continue;
                        }
                    };
                    if msgs.is_empty() {
                        eprintln!("(no messages)");
                    } else {
                        let tty = std::io::stdout().is_terminal();
                        for m in &msgs {
                            match m.role {
                                store::model::ChatRole::User => println!("you> {}", m.content),
                                store::model::ChatRole::Assistant => {
                                    println!("assistant> {}", render_markdown(&m.content, tty))
                                }
                                _ => {}
                            }
                        }
                    }
                    continue;
                }
                "show" => {
                    match &last_reply {
                        Some(msg) if !msg.trim().is_empty() => {
                            let tty = std::io::stdout().is_terminal();
                            println!("assistant> {}", render_markdown(msg, tty));
                        }
                        _ => eprintln!(
                            "(no reply to show yet — /show re-renders the last assistant reply)"
                        ),
                    }
                    continue;
                }
                _ => {
                    if handle_slash_command(stripped, store.as_ref(), &mut session_id).await? {
                        break;
                    }
                    continue;
                }
            }
        }

        match run_repl_turn(&engine, &mut rx, &rl, session_id, text, None).await {
            Ok((s, reply)) => {
                session_id = Some(s);
                if let Some(t) = title.take() {
                    maybe_apply_title(&store, s, Some(&t)).await;
                }
                last_reply = Some(reply);
            }
            Err(e) => eprintln!("{} {e}\n", err_prefix()),
        }
    }

    {
        let mut g = rl.lock().expect("rl mutex poisoned");
        let _ = g.save_history(&histpath);
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct PendingApproval {
    pub(crate) approval_id: uuid::Uuid,
    pub(crate) session_id: uuid::Uuid,
    pub(crate) tool_name: String,
    pub(crate) tool_args_json: String,
    pub(crate) description: String,
    pub(crate) tool_call_id: String,
}

#[derive(Clone)]
pub(crate) struct PendingUserQuestion {
    pub(crate) question: String,
    pub(crate) options: Vec<String>,
    pub(crate) context: Option<String>,
}

/// Run a chat turn (initial `run_chat` or `resume_chat_after_approval`) with a
/// live progress listener + Ctrl-C cancel. Returns the turn result, whether the
/// assistant reply was streamed raw to stdout, and the latest pending approval
/// (if the turn paused on a mutating tool).
pub(crate) async fn run_turn_with_progress<F>(
    engine: &Engine,
    rx: &mut tokio::sync::broadcast::Receiver<AppEvent>,
    json: bool,
    prefix: Option<String>,
    stream_raw: bool,
    turn: F,
) -> Result<(
    ChatTurnResult,
    bool,
    Option<PendingApproval>,
    Option<PendingUserQuestion>,
)>
where
    F: Future<Output = Result<ChatTurnResult>>,
{
    let streamed = Arc::new(AtomicBool::new(false));
    let pending: Arc<std::sync::Mutex<Option<PendingApproval>>> =
        Arc::new(std::sync::Mutex::new(None));
    let pending_question: Arc<std::sync::Mutex<Option<PendingUserQuestion>>> =
        Arc::new(std::sync::Mutex::new(None));
    // Reasoning is only shown in the interactive REPL (which passes a prompt
    // prefix). `--once` is headless and passes `prefix: None` → no reasoning
    // display. No user-facing flag or config is involved.
    let show_reasoning = prefix.is_some();

    let listener = {
        let mut rx = rx.resubscribe();
        let streamed = Arc::clone(&streamed);
        let pending = Arc::clone(&pending);
        let pending_question = Arc::clone(&pending_question);
        let prefix = prefix.clone();
        tokio::spawn(async move {
            let stderr_tty = std::io::stderr().is_terminal();
            let dim = |s: &str| -> String {
                if stderr_tty {
                    format!("\x1b[2m{s}\x1b[0m")
                } else {
                    s.to_string()
                }
            };
            // A single in-place status line (no trailing newline) that we keep
            // overwriting — used for the reasoning tail preview and the thinking
            // heartbeat. Like the TUI reasoning card, we REPLACE on each emit
            // (never append), so a scrolling terminal never reprints accumulated
            // text. `inplace_active` tracks whether such a line is on screen.
            let mut inplace_active = false;
            let mut seen_reasoning = false;
            let mut last_thinking: u64 = 0;
            let mut last_len: usize = 0; // assistant reply bytes already printed
            let mut prefix_printed = false;
            let mut spin: u64 = 0; // Braille spinner frame counter (P1-1)
                                   // Clear the in-place status line so the next output starts fresh.
            macro_rules! clear_inplace {
                () => {{
                    if inplace_active && stderr_tty {
                        eprint!("\r\x1b[K");
                    }
                    inplace_active = false;
                }};
            }
            while let Ok(ev) = rx.recv().await {
                match ev {
                    AppEvent::ChatReply => break,
                    AppEvent::ChatProgress(p) => match p {
                        ChatProgress::AssistantPartial { text } if !json && stream_raw => {
                            clear_inplace!();
                            if text.len() < last_len {
                                last_len = 0;
                                prefix_printed = false;
                            }
                            if text.len() > last_len {
                                let stdout_tty = std::io::stdout().is_terminal();
                                // P0-4: when stdout is piped (not a TTY), stream
                                // incremental reply to stderr and keep stdout
                                // clean for the final result only.
                                if !stdout_tty {
                                    let mut out = std::io::stderr().lock();
                                    let _ = out.write_all(&text.as_bytes()[last_len..]);
                                    let _ = out.flush();
                                    last_len = text.len();
                                    // Do NOT set `streamed` — the final
                                    // assistant reply will be printed to
                                    // stdout after the turn completes.
                                } else {
                                    let mut out = std::io::stdout().lock();
                                    if !prefix_printed {
                                        if let Some(pfx) = prefix.as_deref() {
                                            if stdout_tty {
                                                let _ = out.write_all(
                                                    format!("\x1b[36m{pfx}\x1b[0m").as_bytes(),
                                                );
                                            } else {
                                                let _ = out.write_all(pfx.as_bytes());
                                            }
                                        } else if use_color_stdout() {
                                            let _ = out
                                                .write_all("\x1b[1;36m◆ reply\x1b[0m\n".as_bytes());
                                        }
                                        prefix_printed = true;
                                    }
                                    let _ = out.write_all(&text.as_bytes()[last_len..]);
                                    let _ = out.flush();
                                    last_len = text.len();
                                    streamed.store(true, Ordering::Relaxed);
                                }
                            }
                        }
                        // REPL (stream_raw=false): don't stream the reply to
                        // stdout (that interleaves with stderr events). Instead
                        // show an in-place reply tail preview on stderr — stdout
                        // is inactive here, so `\r\x1b[K` is safe — and print the
                        // full rendered reply once at turn end.
                        ChatProgress::AssistantPartial { text } if show_reasoning && stderr_tty => {
                            let f = spinner_frame(spin);
                            spin = spin.wrapping_add(1);
                            eprint!("\r\x1b[K\x1b[2m{f} {}\x1b[0m", reasoning_tail(&text, 60));
                            inplace_active = true;
                        }
                        // Reasoning tail preview — REPL only (show_reasoning).
                        // Replace on each emit (no append) → no duplication.
                        ChatProgress::ReasoningPartial { text } if show_reasoning && stderr_tty => {
                            seen_reasoning = true;
                            let f = spinner_frame(spin);
                            spin = spin.wrapping_add(1);
                            eprint!("\r\x1b[K\x1b[2m{f} {}\x1b[0m", reasoning_tail(&text, 60));
                            inplace_active = true;
                        }
                        // Heartbeat only before any reasoning streams; once
                        // reasoning flows, the tail preview is the indicator.
                        ChatProgress::TurnThinking { turn, elapsed_secs } if show_reasoning => {
                            if !seen_reasoning
                                && (elapsed_secs == 0 || elapsed_secs >= last_thinking + 5)
                            {
                                last_thinking = elapsed_secs;
                                if stderr_tty {
                                    let f = spinner_frame(spin);
                                    spin = spin.wrapping_add(1);
                                    eprint!(
                                        "\r\x1b[K\x1b[2m{f} thinking (turn {turn}, {elapsed_secs}s)\x1b[0m"
                                    );
                                    inplace_active = true;
                                } else {
                                    eprintln!("… thinking (turn {turn}, {elapsed_secs}s)");
                                }
                            }
                        }
                        ChatProgress::ApprovalQueued {
                            approval_id,
                            session_id,
                            tool_name,
                            tool_args_json,
                            description,
                            tool_call_id,
                        } => {
                            *pending.lock().expect("pending mutex") = Some(PendingApproval {
                                approval_id,
                                session_id,
                                tool_name,
                                tool_args_json,
                                description,
                                tool_call_id,
                            });
                        }
                        ChatProgress::UserQuestionQueued {
                            question,
                            options,
                            context,
                            ..
                        } => {
                            *pending_question.lock().expect("pending question mutex") =
                                Some(PendingUserQuestion {
                                    question,
                                    options,
                                    context,
                                });
                        }
                        // Summarizing streamed reasoning via a think=false LLM call.
                        ChatProgress::ReasoningCompressing if show_reasoning => {
                            clear_inplace!();
                            eprintln!("{}", dim("… summarizing reasoning"));
                        }
                        // `--once` (no reasoning display): swallow the persisted
                        // reasoning-summary line so it never reaches the terminal.
                        ChatProgress::ReasoningSummary { .. } if !json && !show_reasoning => {
                            clear_inplace!();
                        }
                        // P1-3: render tool calls as a distinct block (stderr),
                        // separating them visually from the streamed reply.
                        ChatProgress::ToolStart { name, args_short, .. } if !json => {
                            clear_inplace!();
                            eprintln!(
                                "{}",
                                tool_block_start(name.as_str(), args_short.as_str(), stderr_tty)
                            );
                        }
                        ChatProgress::ToolDone {
                            name,
                            args_short,
                            ok,
                            elapsed_ms,
                            ..
                        } if !json => {
                            clear_inplace!();
                            eprintln!(
                                "{}",
                                tool_block_done(
                                    name.as_str(),
                                    args_short.as_str(),
                                    ok,
                                    elapsed_ms,
                                    stderr_tty
                                )
                            );
                        }
                        other if !json && other.show_in_log() => {
                            clear_inplace!();
                            eprintln!("{}", colorize_progress(&other.display_line(), stderr_tty));
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            // Clear the in-place status line (if any) before the caller prints.
            if inplace_active && stderr_tty {
                eprint!("\r\x1b[K");
            }
        })
    };

    // Ctrl-C cancels the in-flight turn (mirrors TUI Esc) without exiting REPL.
    let cancel_flag = engine.chat_cancel_flag();
    let cancel_task = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            cancel_flag.store(true, Ordering::Relaxed);
            eprintln!("\n^C — cancelling turn…");
        }
    });

    let result = turn.await;
    listener.abort();
    cancel_task.abort();

    while let Ok(ev) = rx.try_recv() {
        if let AppEvent::ChatProgress(p) = ev {
            if !json && p.show_in_log() {
                eprintln!(
                    "{}",
                    colorize_progress(&p.display_line(), std::io::stderr().is_terminal())
                );
            }
        }
    }

    let streamed = streamed.load(Ordering::Relaxed);
    let pending = pending.lock().expect("pending mutex").take();
    let pending_q = pending_question
        .lock()
        .expect("pending question mutex")
        .take();
    result.map(|r| (r, streamed, pending, pending_q))
}

fn print_assistant_reply(result: &ChatTurnResult, streamed: bool) {
    // Clear any leftover in-place reasoning/reply preview on stderr so the
    // rendered reply starts on a fresh line.
    if std::io::stderr().is_terminal() {
        eprint!("\r\x1b[K");
    }
    if !streamed {
        let tty = std::io::stdout().is_terminal();
        println!(
            "assistant> {}",
            render_markdown(&result.assistant_message, tty)
        );
    }
    println!();
}

/// Run one REPL turn (initial or retry) and drive the approval loop to
/// completion, prompting y/n for each mutating tool. Returns the final session
/// id and the last assistant message (for `/show`).
async fn run_repl_turn(
    engine: &Engine,
    rx: &mut tokio::sync::broadcast::Receiver<AppEvent>,
    rl: &Arc<std::sync::Mutex<DefaultEditor>>,
    session_id: Option<uuid::Uuid>,
    message: &str,
    regenerate_from: Option<uuid::Uuid>,
) -> Result<(uuid::Uuid, String)> {
    let run_future: std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ChatTurnResult>> + Send>,
    > = match regenerate_from {
        Some(assistant_id) => {
            let sid = session_id.expect("regenerate requires a session");
            Box::pin(engine.regenerate_chat(sid, assistant_id))
        }
        None => Box::pin(engine.run_chat(session_id, message)),
    };
    let (mut result, streamed, mut pending, mut pending_q) = run_turn_with_progress(
        engine,
        rx,
        false,
        Some("assistant> ".to_string()),
        false,
        run_future,
    )
    .await?;
    print_assistant_reply(&result, streamed);
    let mut sid = result.session_id;
    let mut last_msg = result.assistant_message.clone();

    while result.awaiting_user_input {
        sid = result.session_id;
        let q = pending_q.take();
        if let Some(ref q) = q {
            if std::io::stderr().is_terminal() {
                eprintln!("\n\x1b[36m❓ {}\x1b[0m", q.question);
            } else {
                eprintln!("\n? {}", q.question);
            }
            if let Some(ctx) = &q.context {
                eprintln!("  ({ctx})");
            }
            if !q.options.is_empty() {
                for (i, opt) in q.options.iter().enumerate() {
                    eprintln!("  {}. {opt}", i + 1);
                }
                eprintln!("  Enter a number, or type a custom answer.");
            }
        }
        let prompt = if q.as_ref().is_some_and(|q| !q.options.is_empty()) {
            "answer [n/text]> "
        } else {
            "answer> "
        };
        let Some(raw) = read_repl_line(rl, prompt).await else {
            eprintln!("(no answer — leaving turn paused)");
            break;
        };
        let raw = raw.trim().to_string();
        if raw.is_empty() {
            eprintln!("(empty answer — leaving turn paused)");
            break;
        }
        let answer = resolve_ask_user_answer(q.as_ref(), &raw);
        let (r, s, p, pq) = run_turn_with_progress(
            engine,
            rx,
            false,
            Some("assistant> ".to_string()),
            false,
            engine.run_chat(Some(sid), &answer),
        )
        .await?;
        result = r;
        print_assistant_reply(&result, s);
        last_msg = result.assistant_message.clone();
        pending = p;
        pending_q = pq;
    }

    while result.awaiting_approval {
        let pa = match pending {
            Some(p) => p,
            None => {
                eprintln!("(awaiting approval but no pending info — try `chat --once --yes`)");
                break;
            }
        };
        sid = result.session_id;
        if std::io::stderr().is_terminal() {
            eprintln!(
                "\n\x1b[33m⚠ approval required\x1b[0m — {}: {}",
                pa.tool_name, pa.description
            );
        } else {
            eprintln!("\napproval required — {}: {}", pa.tool_name, pa.description);
        }
        eprintln!(
            "  args: {}",
            coworker_core::agent::redact::redact_json_str(&pa.tool_args_json)
        );
        let approve = prompt_yes_no(rl).await;
        let detail = match engine.decide_approval(&pa.approval_id, approve, None).await {
            Ok(m) => m,
            Err(e) => {
                eprintln!("approval error: {e}");
                e.to_string()
            }
        };
        let tool_args =
            serde_json::from_str(&pa.tool_args_json).unwrap_or_else(|_| serde_json::json!({}));
        let resume = ResumeChatAfterApproval {
            approval_id: pa.approval_id,
            approved: approve,
            detail,
            tool_name: pa.tool_name.clone(),
            tool_args,
            tool_call_id: pa.tool_call_id.clone(),
        };
        let (r, s, p, pq) = run_turn_with_progress(
            engine,
            rx,
            false,
            Some("assistant> ".to_string()),
            false,
            engine.resume_chat_after_approval(pa.session_id, resume),
        )
        .await?;
        result = r;
        print_assistant_reply(&result, s);
        last_msg = result.assistant_message.clone();
        pending = p;
        let _ = pq;
    }
    Ok((sid, last_msg))
}

/// Map a numbered reply to the option text when `ask_user` provided choices.
fn resolve_ask_user_answer(q: Option<&PendingUserQuestion>, raw: &str) -> String {
    let Some(q) = q else {
        return raw.to_string();
    };
    if q.options.is_empty() {
        return raw.to_string();
    }
    if let Ok(n) = raw.parse::<usize>() {
        if (1..=q.options.len()).contains(&n) {
            return q.options[n - 1].clone();
        }
    }
    raw.to_string()
}

/// Read one line via rustyline (sub-prompt, e.g. picker / y-n). None on EOF /
/// interrupt — callers treat that as cancel/deny.
async fn read_repl_line(rl: &Arc<std::sync::Mutex<DefaultEditor>>, prompt: &str) -> Option<String> {
    let rl2 = Arc::clone(rl);
    let prompt = prompt.to_string();
    let res = tokio::task::spawn_blocking(move || {
        let mut g = rl2.lock().expect("rl mutex poisoned");
        g.readline(&prompt)
    })
    .await;
    match res {
        Ok(Ok(line)) => Some(line),
        _ => None,
    }
}

async fn prompt_yes_no(rl: &Arc<std::sync::Mutex<DefaultEditor>>) -> bool {
    loop {
        match read_repl_line(rl, "approve? [y/n] ").await {
            Some(line) => {
                let t = line.trim().to_ascii_lowercase();
                if t.starts_with('y') {
                    return true;
                }
                if t.starts_with('n') {
                    return false;
                }
                eprintln!("  please answer y or n");
            }
            None => return false, // Ctrl-D / cancel → deny
        }
    }
}

pub(crate) async fn list_chat_sessions(
    store: &dyn store::Store,
    json: bool,
    limit: usize,
) -> Result<()> {
    let sessions = store.list_chat_sessions(limit).await?;
    if json {
        emit_json(serde_json::to_value(&sessions)?);
        return Ok(());
    }
    if sessions.is_empty() {
        eprintln!("(no chat sessions)");
        return Ok(());
    }
    let tty = use_color_stdout();
    let mut rows: Vec<Vec<String>> = Vec::new();
    for s in sessions {
        rows.push(vec![
            s.id.to_string(),
            s.created_at.format("%Y-%m-%d %H:%M").to_string(),
            s.title,
        ]);
    }
    println!("{}", table(&["session", "created", "title"], &rows, tty));
    Ok(())
}
/// `/resume [<id|num>]` — no arg opens a numbered picker; a UUID resumes
/// directly; a number picks from the recent list.
async fn handle_resume(
    store: &Arc<dyn store::Store>,
    rl: &Arc<std::sync::Mutex<DefaultEditor>>,
    session_id: &mut Option<uuid::Uuid>,
    arg: Option<String>,
) -> Result<()> {
    let pick_by_index = |sessions: &[store::model::ChatSession], n: usize| -> Option<uuid::Uuid> {
        sessions.get(n.saturating_sub(1)).map(|s| s.id)
    };
    match arg {
        Some(s) => {
            if let Ok(id) = uuid::Uuid::parse_str(&s) {
                *session_id = Some(id);
                eprintln!("(resumed {id})");
                return Ok(());
            }
            match s.parse::<usize>() {
                Ok(n) => {
                    let sessions = store.list_chat_sessions(20).await?;
                    match pick_by_index(&sessions, n) {
                        Some(id) => {
                            *session_id = Some(id);
                            let title = sessions
                                .iter()
                                .find(|x| x.id == id)
                                .map(|x| x.title.clone())
                                .unwrap_or_default();
                            eprintln!("(resumed {id} — {title})");
                        }
                        None => eprintln!("(no session #{n})"),
                    }
                }
                Err(_) => eprintln!("invalid session id or number: {s}"),
            }
        }
        None => {
            let sessions = store.list_chat_sessions(20).await?;
            if sessions.is_empty() {
                eprintln!("(no sessions)");
                return Ok(());
            }
            for (i, sess) in sessions.iter().enumerate() {
                let mark = if Some(sess.id) == *session_id {
                    "*"
                } else {
                    " "
                };
                eprintln!(
                    "{mark} {}. {}  {}",
                    i + 1,
                    sess.created_at.format("%Y-%m-%d %H:%M"),
                    sess.title
                );
            }
            if let Some(line) = read_repl_line(rl, "select> ").await {
                let t = line.trim();
                if let Ok(id) = uuid::Uuid::parse_str(t) {
                    *session_id = Some(id);
                    eprintln!("(resumed {id})");
                } else if let Ok(n) = t.parse::<usize>() {
                    match pick_by_index(&sessions, n) {
                        Some(id) => {
                            *session_id = Some(id);
                            eprintln!("(resumed {id})");
                        }
                        None => eprintln!("(no session #{n})"),
                    }
                } else {
                    eprintln!("invalid selection: {t}");
                }
            }
        }
    }
    Ok(())
}

/// Rename a freshly created/used session when `--title` was supplied.
async fn maybe_apply_title(
    store: &Arc<dyn store::Store>,
    session_id: uuid::Uuid,
    title: Option<&str>,
) {
    if let Some(t) = title {
        if let Ok(Some(mut sess)) = store.get_chat_session(&session_id).await {
            if sess.title != t {
                sess.title = t.to_string();
                let _ = store.update_chat_session(&sess).await;
            }
        }
    }
}

fn cli_history_path(config: &Config) -> PathBuf {
    let sp = config.storage_path();
    let dir = sp.parent().unwrap_or_else(|| Path::new("."));
    dir.join("coworker-cli-history.txt")
}

fn repl_prompt(session_id: Option<uuid::Uuid>) -> String {
    let tty = std::io::stdout().is_terminal();
    let label = match session_id {
        Some(id) => format!("you·{}", &id.to_string()[..6]),
        None => "you".to_string(),
    };
    if tty {
        format!("\x1b[32m{label}\x1b[0m> ")
    } else {
        format!("{label}> ")
    }
}

async fn handle_slash_command(
    cmd: &str,
    store: &dyn store::Store,
    session_id: &mut Option<uuid::Uuid>,
) -> Result<bool> {
    let mut parts = cmd.split_whitespace();
    let name = parts.next().unwrap_or("");
    let _arg = parts.next();
    match name {
        "help" | "h" | "?" => {
            eprintln!("commands:");
            eprintln!("  /help            show this help");
            eprintln!("  /sessions        list recent sessions");
            eprintln!("  /new             start a new session");
            eprintln!("  /resume [<id|n>] resume a session (no arg = numbered picker)");
            eprintln!("  /retry           re-run the last user message");
            eprintln!("  /history [N]     show recent messages (assistant rendered as Markdown)");
            eprintln!("  /show            re-render the last assistant reply as Markdown");
            eprintln!("  /clear           clear the screen");
            eprintln!("  /quit            exit (Ctrl-D also exits)");
        }
        "quit" | "exit" => return Ok(true),
        "sessions" | "s" => {
            let sessions = store.list_chat_sessions(20).await?;
            if sessions.is_empty() {
                eprintln!("(no sessions)");
            } else {
                for s in sessions {
                    let mark = if Some(s.id) == *session_id { "*" } else { " " };
                    eprintln!(
                        "{mark} {}  {}  {}",
                        s.id,
                        s.created_at.format("%Y-%m-%d %H:%M"),
                        s.title
                    );
                }
            }
        }
        "new" => {
            *session_id = None;
            eprintln!("(new session — next message starts it)");
        }
        "clear" | "cls" => {
            print!("\x1b[2J\x1b[3J\x1b[H");
            let _ = std::io::stdout().flush();
        }
        other => eprintln!("unknown command: /{other} (try /help)"),
    }
    Ok(false)
}
