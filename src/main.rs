mod agent;
mod app;
mod approval_payload;
mod config;
mod diagnostics;
mod engine;
mod error;
mod exit_codes;
mod github;
mod llm;
mod logging;
mod mcp;
mod output;
mod store;
mod terminal;
mod tui;
mod web;

use std::future::Future;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use clap::{CommandFactory, Parser, Subcommand};
use pulldown_cmark::{Event, Parser as MdParser, Tag, TagEnd};
use rustyline::config::Configurer;
use rustyline::{ColorMode, DefaultEditor};
use serde::Deserialize;
use tokio::io::AsyncBufReadExt;

use app::{event_channel, hydrate_from_store, AppEvent, AppState, SharedState};
use config::Config;
use engine::Engine;
use error::{CoworkerError, Result};
use store::open_store;

use agent::chat_loop::{ChatProgress, ChatTurnResult, ResumeChatAfterApproval};

#[derive(Parser)]
#[command(
    name = "unistar-coworker",
    about = "Local GitHub ops secretary with TUI",
    after_help = "EXAMPLES:\n    unistar-coworker tui                                  Terminal UI (default)\n    unistar-coworker serve                            Web UI server\n    unistar-coworker chat                             interactive chat REPL\n    unistar-coworker chat --once \"summarize PR 123\" --json\n    unistar-coworker run-once --workflow daily-work\n    unistar-coworker triage-pr --repo acme/widget --pr 42 --json\n    unistar-coworker report oncall\n    unistar-coworker store compact --dry-run --audit-days 30\n\nGlobal flags (--config / -v / -q / --attach) go before the subcommand."
)]
struct Cli {
    /// Attach to daemon store only — do not start a local cron scheduler
    #[arg(long)]
    attach: bool,
    /// Override config file path (skips discover in cwd / .coworker/)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Increase log verbosity (-v = debug, -vv = trace)
    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    /// Decrease log verbosity to warn
    #[arg(short = 'q', long, global = true)]
    quiet: bool,
    /// Disable all ANSI color / box-drawing in output (plain text)
    #[arg(long, global = true)]
    plain: bool,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a workflow once without TUI (default: daily-work)
    #[command(
        after_help = "EXAMPLES:\n    unistar-coworker run-once\n    unistar-coworker run-once --workflow review-radar --json"
    )]
    RunOnce {
        #[arg(long, default_value = "daily-work")]
        workflow: String,
        /// Emit machine-readable JSON on stdout
        #[arg(long)]
        json: bool,
        /// Wall-clock timeout in seconds
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Export store reports without running a full workflow
    Report {
        #[command(subcommand)]
        kind: ReportKind,
    },
    /// Debug triage for a single PR
    #[command(
        after_help = "EXAMPLES:\n    unistar-coworker triage-pr --repo acme/widget --pr 42\n    unistar-coworker triage-pr --repo acme/widget --pr 42 --json"
    )]
    TriagePr {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        pr: u32,
        /// Emit machine-readable JSON on stdout
        #[arg(long)]
        json: bool,
        /// Wall-clock timeout in seconds
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Headless daemon: cron scheduler without TUI
    Daemon {
        /// Write the daemon PID to this file on start (removed on clean exit)
        #[arg(long)]
        pid_file: Option<PathBuf>,
    },
    /// Terminal UI with cron scheduler
    Tui,
    /// Web UI server (browser)
    Serve {
        /// Override config `web.bind` (e.g. 127.0.0.1:8787)
        #[arg(long)]
        bind: Option<String>,
    },
    /// Interactive chat REPL
    #[command(
        after_help = "EXAMPLES:\n    unistar-coworker chat\n    unistar-coworker chat --once \"summarize PR 123\"\n    unistar-coworker chat --once \"summarize PR 123\" --json --yes\n    unistar-coworker chat --session 9950379a-3db7-46ec-98ed-11310014b456\n    unistar-coworker chat --list-sessions --limit 50 --json\n\nREPL slash commands: /help /sessions /new /resume [<id|num>] /retry /history [N] /clear /quit\nCtrl-C cancels the current turn; Ctrl-D exits."
    )]
    Chat {
        /// Single message then exit (script-friendly)
        #[arg(long)]
        once: Option<String>,
        /// Resume an existing chat session
        #[arg(long)]
        session: Option<uuid::Uuid>,
        /// List recent chat sessions and exit
        #[arg(long)]
        list_sessions: bool,
        /// Emit machine-readable JSON (chat --once / --list-sessions)
        #[arg(long)]
        json: bool,
        /// Title for a newly created session (used with --once or first message)
        #[arg(long)]
        title: Option<String>,
        /// Auto-approve every mutating tool (headless --once runs; skips the y/n prompt)
        #[arg(long)]
        yes: bool,
        /// Session list limit for --list-sessions
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Wall-clock timeout in seconds for --once (prevents hanging on a stalled LLM)
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Store maintenance (migrate, compact)
    Store {
        #[command(subcommand)]
        cmd: StoreCommands,
    },
    /// List built-in batch workflows (Rust registry)
    Workflows {
        #[command(subcommand)]
        cmd: CatalogCmd,
    },
    /// List technique skills (skills/*/SKILL.md)
    Skills {
        #[command(subcommand)]
        cmd: CatalogCmd,
    },
    /// Self-check config, GitHub CLI, LLM, MCP servers, and store
    Doctor {
        /// Emit machine-readable JSON on stdout
        #[arg(long)]
        json: bool,
    },
    /// Create a starter coworker.yaml (does not overwrite unless --force)
    Init {
        /// Overwrite an existing coworker.yaml
        #[arg(long)]
        force: bool,
        /// Target path (defaults to ./coworker.yaml)
        #[arg(long)]
        path: Option<PathBuf>,
        /// Comma-separated repos to seed (e.g. acme/widget,acme/api)
        #[arg(long)]
        repos: Option<String>,
        /// LLM base_url to seed (e.g. http://localhost:11434/v1)
        #[arg(long)]
        llm_url: Option<String>,
    },
    /// Export stored data (Pi-style session tree: JSONL + HTML)
    Export {
        #[command(subcommand)]
        target: ExportTarget,
    },
    /// JSONL RPC over stdin/stdout (Pi-style machine protocol: chat / get_state / cancel)
    Rpc {
        /// Resume an existing chat session (auto-created if omitted)
        #[arg(long)]
        session: Option<uuid::Uuid>,
        /// Auto-approve mutating tools instead of pausing for approval
        #[arg(long)]
        yes: bool,
        /// Wall-clock turn timeout in seconds
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Generate shell completion scripts (bash / zsh / fish / powershell)
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
enum ExportTarget {
    /// Export a full chat session (active branch: user/assistant/tool messages)
    Session {
        /// Chat session id
        id: uuid::Uuid,
        /// Output format
        #[arg(long, value_enum, default_value_t = ExportFormat::Jsonl)]
        format: ExportFormat,
        /// Write to this file instead of stdout
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(clap::ValueEnum, Clone, Copy)]
enum ExportFormat {
    Jsonl,
    Html,
}

#[derive(Subcommand)]
enum CatalogCmd {
    /// Print name, path, description
    List {
        /// Emit machine-readable JSON on stdout
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum StoreCommands {
    /// Copy data between json and sqlite backends
    Migrate {
        #[arg(long, default_value = "json")]
        from: String,
        #[arg(long, default_value = "sqlite")]
        to: String,
        #[arg(long)]
        source: String,
        #[arg(long)]
        dest: String,
    },
    /// Prune old audit entries, digests, and workflow runs
    Compact {
        /// Prune audit entries older than N days
        #[arg(long, default_value_t = 90)]
        audit_days: u32,
        /// Keep only the N most recent digests
        #[arg(long, default_value_t = 30)]
        digest_keep: u32,
        /// Prune completed workflow runs finished more than N days ago
        #[arg(long, default_value_t = 30)]
        workflow_runs_days: u32,
        /// Preview what would be pruned without deleting anything
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum ReportKind {
    /// On-call handoff pack from local store (no MCP)
    Oncall {
        /// Wrap the report in a JSON object on stdout
        #[arg(long)]
        json: bool,
    },
    /// CI efficiency report (requires MCP)
    Ci {
        #[arg(long, default_value_t = 7)]
        since_days: u32,
        /// Wrap the report in a JSON object on stdout
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() {
    if let Err(e) = run_cli().await {
        eprintln!("{} {e}", err_prefix());
        std::process::exit(exit_codes::exit_code_for_error(&e));
    }
}

async fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    set_plain(cli.plain);
    let tui_mode = matches!(cli.command, None | Some(Commands::Tui));
    // Interactive chat REPL: sink tracing so INFO/WARN logs don't interleave
    // with the in-place reasoning preview and streamed reply on the terminal.
    // (`chat --once` / `--list-sessions` stay on stderr — they're headless.)
    let chat_repl = matches!(
        cli.command,
        Some(Commands::Chat {
            once: None,
            list_sessions: false,
            ..
        })
    );
    logging::init_tracing(tui_mode, cli.verbose, cli.quiet, chat_repl);

    // --attach is only meaningful for the long-running modes that own a
    // scheduler (TUI / serve); warn if it's passed alongside a subcommand
    // that silently ignores it.
    if cli.attach
        && !matches!(
            cli.command,
            None | Some(Commands::Tui)
                | Some(Commands::Serve { .. })
                | Some(Commands::Daemon { .. })
        )
    {
        eprintln!(
            "{} --attach has no effect for this subcommand (only TUI / serve / daemon consume it)",
            warn_prefix()
        );
    }

    // `doctor`, `init`, and `completions` run without the full config/store load.
    if let Some(Commands::Completions { shell }) = &cli.command {
        let mut cmd = Cli::command();
        clap_complete::generate(*shell, &mut cmd, "unistar-coworker", &mut std::io::stdout());
        return Ok(());
    }
    if let Some(Commands::Doctor { json }) = &cli.command {
        return run_doctor(cli.config.clone(), *json).await;
    }
    if let Some(Commands::Init {
        force,
        path,
        repos,
        llm_url,
    }) = &cli.command
    {
        return run_init(
            *force,
            cli.config.clone(),
            path.clone(),
            repos.clone(),
            llm_url.clone(),
        )
        .await;
    }

    let (config, config_path) = match &cli.config {
        Some(path) => (Config::load(path)?, path.clone()),
        None => Config::discover()?,
    };
    let config_path = config_path.display().to_string();
    let store = Arc::from(open_store(&config)?);

    match cli.command {
        Some(Commands::RunOnce {
            workflow,
            json,
            timeout,
        }) => {
            let run = run_headless(config, store, &workflow, json || cli.quiet);
            let outcome = match timeout {
                Some(secs) => {
                    match tokio::time::timeout(std::time::Duration::from_secs(secs), run).await {
                        Ok(r) => r,
                        Err(_) => {
                            if json {
                                emit_json(
                                    serde_json::json!({ "ok": false, "workflow": workflow, "error": "timeout" }),
                                );
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
                None => run.await,
            };
            match outcome {
                Ok(msg) => {
                    if json {
                        emit_json(
                            serde_json::json!({ "ok": true, "workflow": workflow, "message": msg }),
                        );
                    } else {
                        println!("{msg}");
                    }
                }
                Err(e) => {
                    if json {
                        emit_json(
                            serde_json::json!({ "ok": false, "workflow": workflow, "error": e.to_string() }),
                        );
                    } else {
                        eprintln!("{} {e}", err_prefix());
                    }
                    std::process::exit(exit_codes::EXIT_GENERAL);
                }
            }
        }
        Some(Commands::Report { kind }) => {
            run_report(&config, store.as_ref(), kind).await?;
        }
        Some(Commands::TriagePr {
            repo,
            pr,
            json,
            timeout,
        }) => {
            run_triage_pr(config, store, &repo, pr, json, timeout).await?;
        }
        Some(Commands::Daemon { pid_file }) => {
            run_daemon(config, store, pid_file).await?;
        }
        Some(Commands::Tui) | None => {
            run_tui(config, config_path, store, cli.attach).await?;
        }
        Some(Commands::Serve { bind }) => {
            run_web(config, config_path, store, cli.attach, bind).await?;
        }
        Some(Commands::Chat {
            once,
            session,
            list_sessions,
            json,
            title,
            yes,
            limit,
            timeout,
        }) => {
            if list_sessions {
                list_chat_sessions(store.as_ref(), json, limit).await?;
            } else {
                run_chat_cli(config, store, once, session, json, title, yes, timeout).await?;
            }
        }
        Some(Commands::Store { cmd }) => {
            run_store_cmd(config, cmd).await?;
        }
        Some(Commands::Export { target }) => {
            run_export_cmd(store.as_ref(), target).await?;
        }
        Some(Commands::Rpc {
            session,
            yes,
            timeout,
        }) => {
            run_rpc(config, store, session, yes, timeout).await?;
        }
        Some(Commands::Workflows { cmd }) => {
            run_workflows_list(cmd).await?;
        }
        Some(Commands::Skills { cmd }) => {
            run_catalog_list("skills", "SKILL.md", cmd).await?;
        }
        // Handled by the early returns above (can run without a config).
        Some(Commands::Doctor { .. })
        | Some(Commands::Init { .. })
        | Some(Commands::Completions { .. }) => {
            unreachable!()
        }
    }
    Ok(())
}

async fn run_report(config: &Config, store: &dyn store::Store, kind: ReportKind) -> Result<()> {
    use agent::oncall::build_handoff_markdown;

    let json = match &kind {
        ReportKind::Oncall { json } | ReportKind::Ci { json, .. } => *json,
    };
    let result: Result<(&'static str, String, Option<u32>)> = match kind {
        ReportKind::Oncall { json: _ } => build_handoff_markdown(store)
            .await
            .map(|md| ("oncall", md, None)),
        ReportKind::Ci {
            since_days,
            json: _,
        } => {
            let github = github::spawn_github(config).await;
            agent::ci_efficiency::build_ci_efficiency_markdown(config, github.as_ref())
                .await
                .map(|md| ("ci", md, Some(since_days)))
        }
    };
    match result {
        Ok((kind, md, since)) => {
            if json {
                let mut obj = serde_json::json!({ "ok": true, "kind": kind, "report": md });
                if let Some(s) = since {
                    obj["since_days"] = serde_json::json!(s);
                }
                emit_json(obj);
            } else {
                let tty = use_color_stdout();
                // Render markdown (headings cyan, code dim, rules) on a TTY for a
                // cleaner handoff pack; keep raw markdown when piped.
                println!("{}", render_markdown(&md, tty));
            }
        }
        Err(e) => {
            if json {
                emit_json(
                    serde_json::json!({ "ok": false, "kind": "report", "error": e.to_string() }),
                );
            } else {
                eprintln!("{} {e}", err_prefix());
            }
            std::process::exit(exit_codes::EXIT_GENERAL);
        }
    }
    Ok(())
}

fn parse_storage_backend(name: &str) -> Result<config::StorageBackend> {
    use config::StorageBackend;
    match name.to_ascii_lowercase().as_str() {
        "json" => Ok(StorageBackend::Json),
        "sqlite" => Ok(StorageBackend::Sqlite),
        other => Err(crate::error::CoworkerError::Config(format!(
            "unknown storage backend `{other}` (use json or sqlite)"
        ))),
    }
}

async fn run_store_cmd(config: Config, cmd: StoreCommands) -> Result<()> {
    use config::expand_tilde;
    use store::{compact, migrate, CompactOptions};

    match cmd {
        StoreCommands::Migrate {
            from,
            to,
            source,
            dest,
        } => {
            let from = parse_storage_backend(&from)?;
            let to = parse_storage_backend(&to)?;
            let source_path = expand_tilde(&source);
            let dest_path = expand_tilde(&dest);
            let stats =
                migrate(from, to, source_path, dest_path.clone(), config.storage.wal).await?;
            let tty = use_color_stdout();
            println!("{}", render_migrate_summary(&stats, tty));
            eprintln!(
                "Update coworker.yaml storage.backend to {:?} and storage.path to {}",
                to,
                dest_path.display()
            );
        }
        StoreCommands::Compact {
            audit_days,
            digest_keep,
            workflow_runs_days,
            dry_run,
        } => {
            let opts = CompactOptions {
                audit_days,
                digest_keep,
                workflow_runs_days,
                dry_run,
            };
            let path = config.storage_path();
            let stats = compact(
                config.storage.backend,
                path.clone(),
                config.storage.wal,
                &opts,
            )?;
            let tty = use_color_stdout();
            if dry_run {
                if tty {
                    eprintln!("{}", yellow("⚠ DRY-RUN — nothing was deleted.", true));
                } else {
                    eprintln!("DRY RUN — nothing was deleted.");
                }
                println!("{}", render_compact_summary(&stats, true, tty));
                eprintln!(
                    "(would compact {:?} store at {})",
                    config.storage.backend,
                    path.display()
                );
            } else {
                println!("{}", render_compact_summary(&stats, false, tty));
                eprintln!(
                    "compacted {:?} store at {}",
                    config.storage.backend,
                    path.display()
                );
            }
        }
    }
    Ok(())
}

/// Export a chat session (Pi-style session tree) to JSONL or a standalone HTML
/// file. Uses the active branch so a forked conversation exports as a coherent
/// transcript rather than the flat message log.
async fn run_export_cmd(store: &dyn store::Store, target: ExportTarget) -> Result<()> {
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
        let content = crate::agent::redact::redact_json_str(&m.content);
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

/// JSONL RPC server: read one JSON request per line from stdin, write one or
/// more JSON response lines to stdout (Pi-style machine protocol). Requests:
/// `{"op":"chat","message":"..."}`, `{"op":"get_state"}`, `{"op":"cancel"}`,
/// `{"op":"switch_profile","profile":"fast"}`.
async fn run_rpc(
    config: Config,
    store: Arc<dyn store::Store>,
    session: Option<uuid::Uuid>,
    yes: bool,
    timeout: Option<u64>,
) -> Result<()> {
    if !config.chat.enabled {
        return Err(CoworkerError::Workflow(
            "chat disabled — set chat.enabled: true in coworker.yaml".into(),
        ));
    }
    let (tx, _rx) = event_channel();
    let event_tx = tx.clone();
    let state: SharedState = Arc::new(tokio::sync::RwLock::new(AppState::new(
        config.clone(),
        "rpc".into(),
    )));
    let engine = Arc::new(Engine::new(config, Arc::clone(&store), tx, Arc::clone(&state)).await);
    let mut rx = event_tx.subscribe();

    let progress_tx = event_tx.clone();
    tokio::spawn(async move {
        let mut prx = progress_tx.subscribe();
        while let Ok(ev) = prx.recv().await {
            if let crate::app::AppEvent::ChatProgress(p) = ev {
                if let Some(line) = rpc_progress_json(&p) {
                    println!("{line}");
                }
            }
        }
    });

    let mut session_id = session;
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| CoworkerError::Workflow(e.to_string()))?
    {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req: RpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                emit_json(serde_json::json!({
                    "type": "error",
                    "code": "bad_request",
                    "error": e.to_string()
                }));
                continue;
            }
        };
        match req.op.as_str() {
            "chat" => {
                let msg = req.message.unwrap_or_default();
                if let Err(e) =
                    run_rpc_turn(&engine, &mut rx, &mut session_id, &msg, yes, timeout).await
                {
                    emit_json(serde_json::json!({
                        "type": "error",
                        "code": "turn_failed",
                        "error": e.to_string()
                    }));
                }
            }
            "get_state" => {
                let s = state.read().await;
                let snap = crate::web::snapshot::build_snapshot_from(&s);
                emit_json(serde_json::json!({ "type": "state", "snapshot": snap }));
            }
            "cancel" => {
                engine.request_chat_cancel();
                emit_json(serde_json::json!({ "type": "cancelled" }));
            }
            "switch_profile" => match engine.switch_llm_profile(&req.profile).await {
                Ok(()) => emit_json(serde_json::json!({
                    "type": "profile",
                    "profile": req.profile
                })),
                Err(e) => emit_json(serde_json::json!({
                    "type": "error",
                    "code": "profile",
                    "error": e.to_string()
                })),
            },
            other => emit_json(serde_json::json!({
                "type": "error",
                "code": "unknown_op",
                "op": other
            })),
        }
    }
    Ok(())
}

async fn run_rpc_turn(
    engine: &Arc<Engine>,
    rx: &mut tokio::sync::broadcast::Receiver<crate::app::AppEvent>,
    session_id: &mut Option<uuid::Uuid>,
    msg: &str,
    yes: bool,
    timeout: Option<u64>,
) -> Result<()> {
    let run_once = async {
        let (mut result, _streamed, mut pending) = run_turn_with_progress(
            engine,
            rx,
            true,
            None,
            false,
            engine.run_chat(*session_id, msg),
        )
        .await?;
        while result.awaiting_approval {
            let pa = match pending {
                Some(p) => p,
                None => break,
            };
            if !yes {
                emit_json(serde_json::json!({
                    "type": "error",
                    "code": "approval_required",
                    "session_id": result.session_id,
                    "pending_approval": {
                        "tool": pa.tool_name,
                        "args": crate::agent::redact::redact_json_str(&pa.tool_args_json),
                        "description": pa.description,
                    }
                }));
                break;
            }
            let detail = engine
                .decide_approval(&pa.approval_id, true)
                .await
                .unwrap_or_default();
            let tool_args =
                serde_json::from_str(&pa.tool_args_json).unwrap_or_else(|_| serde_json::json!({}));
            let resume = crate::agent::chat_loop::ResumeChatAfterApproval {
                approval_id: pa.approval_id,
                approved: true,
                detail,
                tool_name: pa.tool_name.clone(),
                tool_args,
            };
            let (r, _s, p) = run_turn_with_progress(
                engine,
                rx,
                true,
                None,
                false,
                engine.resume_chat_after_approval(pa.session_id, resume),
            )
            .await?;
            result = r;
            pending = p;
        }
        Ok::<_, CoworkerError>(result)
    };
    let result = match timeout {
        Some(secs) => {
            match tokio::time::timeout(std::time::Duration::from_secs(secs), run_once).await {
                Ok(r) => r?,
                Err(_) => {
                    emit_json(serde_json::json!({ "type": "error", "code": "timeout" }));
                    return Ok(());
                }
            }
        }
        None => run_once.await?,
    };
    emit_json(serde_json::json!({
        "type": "result",
        "ok": true,
        "session_id": result.session_id,
        "assistant": result.assistant_message,
        "tool_calls": result
            .tool_calls
            .iter()
            .map(|tc| serde_json::json!({ "tool": tc.tool_name, "output": tc.output }))
            .collect::<Vec<_>>(),
        "awaiting_approval": result.awaiting_approval,
    }));
    Ok(())
}

#[derive(Deserialize)]
struct RpcRequest {
    op: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    profile: String,
}

/// Map a streaming `ChatProgress` event to a single-line JSON progress record
/// for the RPC protocol (returns `None` for events with no RPC-relevant info).
fn rpc_progress_json(p: &crate::agent::chat_loop::ChatProgress) -> Option<String> {
    use crate::agent::chat_loop::ChatProgress;
    let v = match p {
        ChatProgress::TurnThinking { turn, elapsed_secs } => {
            serde_json::json!({"stage": "thinking", "turn": turn, "elapsed_secs": elapsed_secs})
        }
        ChatProgress::ToolStart { name, args_short } => {
            serde_json::json!({"stage": "tool_start", "name": name, "args": args_short})
        }
        ChatProgress::ToolDone {
            name,
            ok,
            elapsed_ms,
            ..
        } => {
            serde_json::json!({"stage": "tool_done", "name": name, "ok": ok, "elapsed_ms": elapsed_ms})
        }
        ChatProgress::AssistantPartial { text } => {
            serde_json::json!({"stage": "assistant", "text": text})
        }
        ChatProgress::ReasoningPartial { text } => {
            serde_json::json!({"stage": "reasoning", "text": text})
        }
        ChatProgress::ApprovalQueued {
            tool_name,
            description,
            ..
        } => {
            serde_json::json!({"stage": "approval", "tool": tool_name, "description": description})
        }
        ChatProgress::ApprovalResolved {
            tool_name,
            approved,
            ..
        } => {
            serde_json::json!({"stage": "approval_resolved", "tool": tool_name, "approved": approved})
        }
        ChatProgress::ReasoningSummary { preview, .. } => {
            serde_json::json!({"stage": "reasoning_summary", "preview": preview})
        }
        ChatProgress::ActivityFlow { text, .. } => {
            serde_json::json!({"stage": "activity", "text": text})
        }
        _ => return None,
    };
    Some(serde_json::to_string(&serde_json::json!({"type": "progress", "progress": v})).unwrap())
}

/// Two-column, colored summary of a store migration (P0-3).
fn render_migrate_summary(stats: &store::MigrateStats, tty: bool) -> String {
    table(
        &["category", "count"],
        &[
            vec!["digests".into(), stats.digests.to_string()],
            vec!["pr_snapshots".into(), stats.pr_snapshots.to_string()],
            vec!["approvals".into(), stats.approvals.to_string()],
            vec!["backport_items".into(), stats.backport_items.to_string()],
            vec!["chat_messages".into(), stats.chat_messages.to_string()],
        ],
        tty,
    )
}

/// Two-column summary with a proportion bar per category (P0-3 + P1-2).
fn render_compact_summary(stats: &store::CompactStats, dry_run: bool, tty: bool) -> String {
    let rows = [
        ("audit entries", stats.audit_entries_removed),
        ("audit files", stats.audit_files_removed),
        ("digests", stats.digests_removed),
        ("workflow runs", stats.workflow_runs_removed),
    ];
    let max = rows.iter().map(|(_, n)| *n).max().unwrap_or(0).max(1);
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    for (label, n) in rows {
        let bar = if tty {
            progress_bar((n as f64 / max as f64) * 100.0, 12, true)
        } else {
            String::new()
        };
        table_rows.push(vec![label.into(), n.to_string(), bar]);
    }
    let title = if dry_run { "would remove" } else { "removed" };
    if tty {
        format!(
            "{}\n{}",
            bold(title, true),
            table(&["category", "count", "bar"], &table_rows, tty)
        )
    } else {
        table(&["category", "count"], &table_rows, tty)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// `doctor` — self-check config / gh / LLM / MCP / store (shared with Web API).
// ─────────────────────────────────────────────────────────────────────────────

async fn run_doctor(config_override: Option<PathBuf>, json: bool) -> Result<()> {
    let report = diagnostics::run_checks(config_override).await;
    let tty = use_color_stdout();
    if json {
        emit_json(serde_json::to_value(&report).unwrap_or_default());
    } else {
        for c in &report.checks {
            let icon = match c.status {
                "ok" => green("✓", tty),
                "warn" => yellow("⚠", tty),
                _ => red("✗", tty),
            };
            println!("{icon} {:<8} {}", c.name, c.detail);
            if let Some(hint) = &c.hint {
                if c.status == "fail" {
                    println!("         {} {hint}", hint_prefix());
                }
            }
        }
        println!(
            "{} {} ok, {} warn, {} fail",
            bold("summary:", tty),
            report.ok,
            report.warn,
            report.fail
        );
    }
    if report.has_failures() {
        std::process::exit(exit_codes::EXIT_CONFIG);
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// `init` — create a starter coworker.yaml (P1-1).
// ─────────────────────────────────────────────────────────────────────────────

async fn run_init(
    force: bool,
    config_override: Option<PathBuf>,
    path: Option<PathBuf>,
    repos: Option<String>,
    llm_url: Option<String>,
) -> Result<()> {
    let target = path
        .or(config_override)
        .unwrap_or_else(|| PathBuf::from("coworker.yaml"));
    if target.exists() && !force {
        eprintln!(
            "{} already exists — use --force to overwrite",
            target.display()
        );
        return Ok(());
    }

    let template = include_str!("../coworker.example.yaml");
    let mut lines: Vec<String> = template.lines().map(String::from).collect();

    if let Some(repos) = &repos {
        if let Some(idx) = lines.iter().position(|l| l.trim() == "repos:") {
            let j = idx + 1;
            while j < lines.len() && lines[j].starts_with("  - ") {
                lines.remove(j);
            }
            for (k, r) in repos.split(',').enumerate() {
                let r = r.trim();
                if !r.is_empty() {
                    lines.insert(idx + 1 + k, format!("  - {r}"));
                }
            }
        }
    }
    if let Some(url) = &llm_url {
        if let Some(idx) = lines.iter().position(|l| {
            let t = l.trim_start();
            t.starts_with("base_url:") && !t.starts_with('#')
        }) {
            lines[idx] = format!("  base_url: {url}");
        }
    }

    std::fs::write(&target, lines.join("\n"))?;
    let tty = use_color_stdout();
    println!("{} created {}", green("◆", tty), target.display());
    eprintln!(
        "  {} edit `repos:` and `llm.base_url`, then run 'unistar-coworker doctor' to verify",
        hint_prefix()
    );
    Ok(())
}

async fn run_workflows_list(cmd: CatalogCmd) -> Result<()> {
    use engine::WORKFLOWS;

    let CatalogCmd::List { json } = cmd;
    if json {
        let items: Vec<_> = WORKFLOWS
            .iter()
            .map(|wf| {
                serde_json::json!({
                    "id": wf.id,
                    "description": wf.description,
                    "skills": wf.default_skills,
                })
            })
            .collect();
        emit_json(serde_json::json!(items));
    } else {
        let tty = use_color_stdout();
        let mut rows: Vec<Vec<String>> = Vec::new();
        for wf in WORKFLOWS {
            let skills = if wf.default_skills.is_empty() {
                "—".into()
            } else {
                wf.default_skills.join(", ")
            };
            rows.push(vec![wf.id.to_string(), wf.description.to_string(), skills]);
        }
        println!("{}", table(&["id", "description", "skills"], &rows, tty));
    }
    Ok(())
}

async fn run_catalog_list(root: &str, leaf: &str, cmd: CatalogCmd) -> Result<()> {
    use engine::{load_markdown_spec, load_skill_with_base};
    use std::path::Path;

    let CatalogCmd::List { json } = cmd;
    let root_path = Path::new(root);
    if !root_path.is_dir() {
        if json {
            emit_json(serde_json::json!([]));
        } else {
            eprintln!("(no {root}/ directory)");
        }
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(root_path)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut json_items: Vec<serde_json::Value> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('_') {
            continue;
        }
        let path = entry.path().join(leaf);
        if !path.is_file() {
            continue;
        }
        match if root == "skills" {
            load_skill_with_base(&path)
        } else {
            load_markdown_spec(&path)
        } {
            Ok(spec) => {
                let title = if spec.name.is_empty() {
                    name.clone()
                } else {
                    spec.name
                };
                let desc = if spec.description.is_empty() {
                    "—".into()
                } else {
                    spec.description
                };
                let skills = spec.skill_refs.join(", ");
                if json {
                    json_items.push(serde_json::json!({
                        "name": title,
                        "path": path.display().to_string(),
                        "description": desc,
                        "skills": spec.skill_refs,
                    }));
                } else {
                    rows.push(vec![
                        title,
                        path.display().to_string(),
                        desc,
                        if skills.is_empty() {
                            "—".into()
                        } else {
                            skills
                        },
                    ]);
                }
            }
            Err(e) => {
                eprintln!("{}: {e}", path.display());
            }
        }
    }
    if json {
        emit_json(serde_json::json!(json_items));
    } else {
        let tty = use_color_stdout();
        println!(
            "{}",
            table(&["name", "path", "description", "skills"], &rows, tty)
        );
    }
    Ok(())
}

async fn run_triage_pr(
    config: Config,
    store: Arc<dyn store::Store>,
    repo: &str,
    pr_number: u32,
    json: bool,
    timeout: Option<u64>,
) -> Result<()> {
    use agent::parse::parse_pr_line;
    use agent::triage::triage_pr;
    use engine::load_classify_skills_for_triage;

    let github = github::spawn_github(&config).await;
    let llm_online = llm::ollama::probe(&config.llm).await;
    let llm = llm::LlmClient::new(config.llm.clone(), llm_online);
    let classify_skills = load_classify_skills_for_triage(&[])?;

    let list_text = github::helpers::gh_tool(
        github.as_ref(),
        "pr_list_open",
        serde_json::json!({ "repo": repo, "limit": 50 }),
    )
    .await?;

    let pr_line = list_text
        .lines()
        .find_map(|line| {
            let p = parse_pr_line(line)?;
            (p.number == pr_number).then_some(p)
        })
        .ok_or_else(|| {
            crate::error::CoworkerError::Workflow(format!("PR #{pr_number} not found in {repo}"))
        })?;

    let triage_fut = triage_pr(
        &config,
        github.as_ref(),
        &llm,
        store.as_ref(),
        &classify_skills,
        repo,
        &pr_line,
        None,
    );
    let outcome = match timeout {
        Some(secs) => {
            match tokio::time::timeout(std::time::Duration::from_secs(secs), triage_fut).await {
                Ok(r) => r?,
                Err(_) => {
                    if json {
                        emit_json(
                            serde_json::json!({ "ok": false, "repo": repo, "pr": pr_number, "error": "timeout" }),
                        );
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
        None => triage_fut.await?,
    };

    if json {
        let runs: Vec<_> = outcome
            .runs
            .iter()
            .map(|r| {
                serde_json::json!({
                    "verdict": format!("{:?}", r.verdict),
                    "lines": r.lines,
                })
            })
            .collect();
        emit_json(serde_json::json!({
            "ok": true,
            "repo": repo,
            "pr": pr_number,
            "preamble": outcome.preamble,
            "fallback_attention": outcome.fallback_attention,
            "runs": runs,
        }));
    } else {
        let tty = use_color_stdout();
        println!(
            "{}",
            panel(
                &format!("◆ Triage {repo}#{pr_number}"),
                &outcome
                    .preamble
                    .iter()
                    .map(|l| l.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                tty
            )
        );
        for run in &outcome.runs {
            let verdict = format!("{:?}", run.verdict);
            let colored = if verdict.to_lowercase().starts_with("pass") {
                green(&verdict, tty)
            } else if verdict.to_lowercase().starts_with("fail") {
                red(&verdict, tty)
            } else {
                yellow(&verdict, tty)
            };
            println!("\n{} {}", bold("verdict:", tty), colored);
            for line in &run.lines {
                println!("{line}");
            }
        }
    }
    Ok(())
}

async fn run_headless(
    config: Config,
    store: Arc<dyn store::Store>,
    workflow: &str,
    quiet: bool,
) -> Result<String> {
    let (tx, mut rx) = event_channel();
    let state: SharedState = Arc::new(tokio::sync::RwLock::new(AppState::new(
        config.clone(),
        "headless".into(),
    )));
    let engine = Arc::new(Engine::new(config, Arc::clone(&store), tx, Arc::clone(&state)).await);

    let printer = tokio::spawn(async move {
        while let Ok(ev) = rx.recv().await {
            if quiet {
                // --json: keep stdout clean and avoid stderr progress noise.
                continue;
            }
            match ev {
                AppEvent::LogLine(l) => eprintln!("[{}] {}", l.level, l.message),
                AppEvent::WorkflowStarted { workflow_id } => {
                    eprintln!("→ {workflow_id} started");
                }
                AppEvent::WorkflowFinished {
                    workflow_id,
                    ok,
                    message,
                } => {
                    let status = if ok { "done" } else { "failed" };
                    eprintln!("→ {workflow_id} {status}: {message}");
                }
                AppEvent::DigestReady(d) => {
                    let label = if d.summary.complete {
                        "digest ready"
                    } else {
                        "digest updated"
                    };
                    if d.body_md.contains("Review Radar") {
                        eprintln!(
                            "→ {label} (waiting:{} in {})",
                            d.summary.ignorable,
                            d.summary.duration_label()
                        );
                    } else {
                        eprintln!(
                            "→ {label} (attention:{} flaky:{} in {})",
                            d.summary.needs_attention,
                            d.summary.flaky_candidates,
                            d.summary.duration_label()
                        );
                    }
                }
                AppEvent::StoreUpdated
                | AppEvent::StatusMessage(_)
                | AppEvent::ChatReply
                | AppEvent::ChatProgress(_)
                | AppEvent::PrOverviewReady { .. } => {}
            }
        }
    });

    let msg = engine.run_workflow(workflow).await?;
    printer.abort();
    Ok(msg)
}

/// Spawn a task that hot-reloads config/LLM/MCP on SIGHUP (Pi-style `/reload`
/// without restart). No-op on non-unix platforms.
#[cfg(unix)]
fn spawn_sighup_reload(engine: Arc<Engine>) {
    use tokio::signal::unix::{signal, SignalKind};
    tokio::spawn(async move {
        match signal(SignalKind::hangup()) {
            Ok(mut s) => {
                while s.recv().await.is_some() {
                    engine.reload_all().await;
                }
            }
            Err(e) => tracing::warn!("SIGHUP handler init failed: {e}"),
        }
    });
}

#[cfg(not(unix))]
fn spawn_sighup_reload(_engine: Arc<Engine>) {}

async fn run_daemon(
    config: Config,
    store: Arc<dyn store::Store>,
    pid_file: Option<PathBuf>,
) -> Result<()> {
    let (tx, _rx) = event_channel();
    let state: SharedState = Arc::new(tokio::sync::RwLock::new(AppState::new(
        config.clone(),
        "daemon".into(),
    )));
    let engine = Arc::new(Engine::new(config, Arc::clone(&store), tx, Arc::clone(&state)).await);
    spawn_sighup_reload(Arc::clone(&engine));
    engine.clone().spawn_background();
    engine.clone().spawn_scheduler();

    if let Some(p) = &pid_file {
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        std::fs::write(p, format!("{}\n", std::process::id()))?;
    }

    eprintln!("unistar-coworker daemon started (cron scheduler active; Ctrl-C / SIGTERM to stop)");
    // Graceful shutdown on SIGINT *or* SIGTERM (unix).
    let sigterm = async {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            if let Ok(mut s) = signal(SignalKind::terminate()) {
                s.recv().await;
            } else {
                std::future::pending::<()>().await;
            }
        }
        #[cfg(not(unix))]
        {
            std::future::pending::<()>().await;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm => {}
    }
    eprintln!("daemon shutting down");
    if let Some(p) = &pid_file {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_chat_cli(
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
        return Err(crate::error::CoworkerError::Workflow(
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
            let (mut result, mut streamed, mut pending) = run_turn_with_progress(
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
                                    "args": crate::agent::redact::redact_json_str(&pa.tool_args_json),
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
                    .decide_approval(&pa.approval_id, true)
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
                };
                let (r, s, p) = run_turn_with_progress(
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
            Ok::<_, crate::error::CoworkerError>((result, streamed))
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
        crate::error::CoworkerError::Workflow(format!("rustyline init failed: {e}"))
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
                    let session = match store.get_chat_session(&sid).await {
                        Ok(Some(s)) => s,
                        _ => {
                            eprintln!("(session not found)");
                            continue;
                        }
                    };
                    let branch = match store.list_active_branch_messages(&session, 200).await {
                        Ok(m) => m,
                        Err(e) => {
                            eprintln!("{} {e}", err_prefix());
                            continue;
                        }
                    };
                    let last_assistant = branch
                        .iter()
                        .rev()
                        .find(|m| m.role == store::model::ChatRole::Assistant)
                        .map(|m| m.id);
                    match last_assistant {
                        Some(aid) => {
                            eprintln!("(regenerating branch from assistant {aid})");
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
struct PendingApproval {
    approval_id: uuid::Uuid,
    session_id: uuid::Uuid,
    tool_name: String,
    tool_args_json: String,
    description: String,
}

/// Run a chat turn (initial `run_chat` or `resume_chat_after_approval`) with a
/// live progress listener + Ctrl-C cancel. Returns the turn result, whether the
/// assistant reply was streamed raw to stdout, and the latest pending approval
/// (if the turn paused on a mutating tool).
async fn run_turn_with_progress<F>(
    engine: &Engine,
    rx: &mut tokio::sync::broadcast::Receiver<AppEvent>,
    json: bool,
    prefix: Option<String>,
    stream_raw: bool,
    turn: F,
) -> Result<(ChatTurnResult, bool, Option<PendingApproval>)>
where
    F: Future<Output = Result<ChatTurnResult>>,
{
    let streamed = Arc::new(AtomicBool::new(false));
    let pending: Arc<std::sync::Mutex<Option<PendingApproval>>> =
        Arc::new(std::sync::Mutex::new(None));
    // Reasoning is only shown in the interactive REPL (which passes a prompt
    // prefix). `--once` is headless and passes `prefix: None` → no reasoning
    // display. No user-facing flag or config is involved.
    let show_reasoning = prefix.is_some();

    let listener = {
        let mut rx = rx.resubscribe();
        let streamed = Arc::clone(&streamed);
        let pending = Arc::clone(&pending);
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
                        } => {
                            *pending.lock().expect("pending mutex") = Some(PendingApproval {
                                approval_id,
                                session_id,
                                tool_name,
                                tool_args_json,
                                description,
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
                        ChatProgress::ToolStart { name, args_short } if !json => {
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
    result.map(|r| (r, streamed, pending))
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
    let (mut result, streamed, mut pending) = run_turn_with_progress(
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
            crate::agent::redact::redact_json_str(&pa.tool_args_json)
        );
        let approve = prompt_yes_no(rl).await;
        let detail = match engine.decide_approval(&pa.approval_id, approve).await {
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
        };
        let (r, s, p) = run_turn_with_progress(
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
    }
    Ok((sid, last_msg))
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

async fn list_chat_sessions(store: &dyn store::Store, json: bool, limit: usize) -> Result<()> {
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

// ─────────────────────────────────────────────────────────────────────────────
// CLI display helpers — TTY-aware, zero new dependencies (reuse crossterm/ANSI).
// All color/box-drawing is gated behind `use_color_*()`, which is false when the
// output is not a terminal OR `--plain` was passed, so pipes/files stay clean.
// ─────────────────────────────────────────────────────────────────────────────

/// Global `--plain` flag: when set, suppress all ANSI (tables also degrade to
/// tab-separated text). Set once in `main()` from `cli.plain`.
static PLAIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn set_plain(p: bool) {
    PLAIN.store(p, Ordering::Relaxed);
}
fn use_color_stdout() -> bool {
    std::io::stdout().is_terminal() && !PLAIN.load(Ordering::Relaxed)
}
fn use_color_stderr() -> bool {
    std::io::stderr().is_terminal() && !PLAIN.load(Ordering::Relaxed)
}

/// Wrap `s` in an ANSI SGR sequence when `tty`, else return it untouched.
fn ansi(seq: &str, s: &str, tty: bool) -> String {
    if tty {
        format!("\x1b[{seq}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}
fn cyan(s: &str, tty: bool) -> String {
    ansi("36", s, tty)
}
fn green(s: &str, tty: bool) -> String {
    ansi("32", s, tty)
}
fn red(s: &str, tty: bool) -> String {
    ansi("31", s, tty)
}
fn yellow(s: &str, tty: bool) -> String {
    ansi("33", s, tty)
}
fn purple(s: &str, tty: bool) -> String {
    ansi("35", s, tty)
}
fn dim(s: &str, tty: bool) -> String {
    ansi("2", s, tty)
}
fn bold(s: &str, tty: bool) -> String {
    ansi("1", s, tty)
}

/// Display width that treats CJK-range codepoints as width 2 (no extra deps).
fn disp_width(s: &str) -> usize {
    s.chars()
        .map(|c| if (c as u32) >= 0x1100 { 2 } else { 1 })
        .sum()
}

/// A box-drawing table. On a TTY renders `┌─┬─┐` borders with aligned columns;
/// otherwise degrades to tab-separated rows (script-friendly).
fn table(headers: &[&str], rows: &[Vec<String>], tty: bool) -> String {
    if !tty {
        let mut out = String::new();
        out.push_str(&headers.join("\t"));
        out.push('\n');
        for r in rows {
            out.push_str(&r.join("\t"));
            out.push('\n');
        }
        return out;
    }
    let cols = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| disp_width(h)).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            if i < cols {
                widths[i] = widths[i].max(disp_width(c));
            }
        }
    }
    let mut out = String::new();
    out.push_str(&hbar(&widths, '┌', '┬', '┐'));
    out.push_str(&row_line(headers, &widths, true));
    out.push_str(&hbar(&widths, '├', '┼', '┤'));
    for r in rows {
        out.push_str(&row_line(r, &widths, false));
    }
    out.push_str(&hbar(&widths, '└', '┴', '┘'));
    out
}

fn row_line(cells: &[impl AsRef<str>], widths: &[usize], header: bool) -> String {
    let mut s = String::from("│ ");
    for (i, w) in widths.iter().enumerate() {
        let cell = cells.get(i).map(|c| c.as_ref()).unwrap_or("");
        let padded = format!("{:<width$}", cell, width = w);
        if header {
            s.push_str(&bold(&padded, true));
        } else {
            s.push_str(&padded);
        }
        s.push_str(" │ ");
    }
    s.push('\n');
    s
}

fn hbar(widths: &[usize], l: char, m: char, r: char) -> String {
    let mut s = String::from(l);
    for (i, w) in widths.iter().enumerate() {
        s.push_str(&"─".repeat(w + 2));
        s.push(if i + 1 == widths.len() { r } else { m });
    }
    s.push('\n');
    s
}

/// A titled box panel for a short block of text.
fn panel(title: &str, body: &str, tty: bool) -> String {
    if !tty {
        return format!("{title}\n{body}\n");
    }
    let lines: Vec<&str> = body.lines().collect();
    let inner_w = lines
        .iter()
        .map(|l| disp_width(l))
        .max()
        .unwrap_or(0)
        .max(disp_width(title))
        .max(8);
    let mut s = String::new();
    let title_pad = inner_w.saturating_sub(disp_width(title));
    s.push('┌');
    s.push_str(&format!("─ {title}"));
    s.push_str(&"─".repeat(title_pad + 1));
    s.push('┐');
    s.push('\n');
    for l in lines {
        s.push_str(&format!("│ {:<width$} │\n", l, width = inner_w));
    }
    s.push('└');
    s.push_str(&"─".repeat(inner_w + 2));
    s.push('┘');
    s.push('\n');
    s
}

/// ANSI percentage bar: `███░░░ 42%`. Plain mode returns `42%`.
fn progress_bar(pct: f64, width: usize, tty: bool) -> String {
    if !tty {
        return format!("{pct:.0}%");
    }
    let pct = pct.clamp(0.0, 100.0);
    let filled = ((pct / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let bar: String = "█".repeat(filled);
    let empty: String = "░".repeat(width - filled);
    format!("{}{} {:.0}%", green(&bar, true), dim(&empty, true), pct)
}

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
fn spinner_frame(i: u64) -> char {
    SPINNER[(i as usize) % SPINNER.len()]
}

/// Pretty-print a JSON value to stdout with ANSI syntax highlighting (TTY only).
/// Non-TTY (or `--plain`) emits a single compact line for piping.
fn emit_json(v: serde_json::Value) {
    if use_color_stdout() {
        println!("{}", highlight_json(&v, 0));
    } else {
        println!(
            "{}",
            serde_json::to_string(&v).unwrap_or_else(|_| v.to_string())
        );
    }
}

fn highlight_json(v: &serde_json::Value, indent: usize) -> String {
    let pad = "  ".repeat(indent);
    let pad1 = "  ".repeat(indent + 1);
    match v {
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                return "{}".to_string();
            }
            let mut s = String::from("{\n");
            for (i, (k, val)) in map.iter().enumerate() {
                s.push_str(&pad1);
                s.push_str(&cyan(&format!("\"{k}\""), true));
                s.push_str(": ");
                s.push_str(&highlight_json(val, indent + 1));
                if i + 1 < map.len() {
                    s.push(',');
                }
                s.push('\n');
            }
            s.push_str(&pad);
            s.push('}');
            s
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                return "[]".to_string();
            }
            let mut s = String::from("[\n");
            for (i, val) in arr.iter().enumerate() {
                s.push_str(&pad1);
                s.push_str(&highlight_json(val, indent + 1));
                if i + 1 < arr.len() {
                    s.push(',');
                }
                s.push('\n');
            }
            s.push_str(&pad);
            s.push(']');
            s
        }
        serde_json::Value::String(x) => green(&format!("\"{x}\""), true),
        serde_json::Value::Number(n) => yellow(&n.to_string(), true),
        serde_json::Value::Bool(b) => purple(&b.to_string(), true),
        serde_json::Value::Null => dim("null", true),
    }
}

/// `error:` prefix, red on a TTY (respects `--plain`).
fn err_prefix() -> String {
    red("error:", use_color_stderr())
}

/// `warning:` prefix, yellow on a TTY (respects `--plain`).
fn warn_prefix() -> String {
    yellow("warning:", use_color_stderr())
}

/// `hint:` prefix, cyan on a TTY (respects `--plain`).
fn hint_prefix() -> String {
    cyan("hint:", use_color_stderr())
}

/// `⏱ timeout:` prefix, yellow on a TTY (respects `--plain`).
fn timeout_prefix() -> String {
    if use_color_stderr() {
        format!("{} timeout:", yellow("⏱", true))
    } else {
        "timeout:".to_string()
    }
}

/// Colorize a `ChatProgress::display_line()` for the terminal: the leading
/// marker (`→` cyan, `✓` green, `✗` red, `⚠`/`⏳` yellow) is colored and the
/// remainder dimmed. Plain text when stderr is not a TTY.
fn colorize_progress(line: &str, tty: bool) -> String {
    if !tty {
        return line.to_string();
    }
    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    let rest = &line[indent_len..];
    let (marker, after) = match rest.chars().next() {
        Some('→') => ("\x1b[36m→\x1b[0m", &rest['→'.len_utf8()..]),
        Some('✓') => ("\x1b[32m✓\x1b[0m", &rest['✓'.len_utf8()..]),
        Some('✗') => ("\x1b[31m✗\x1b[0m", &rest['✗'.len_utf8()..]),
        Some('⚠') => ("\x1b[33m⚠\x1b[0m", &rest['⚠'.len_utf8()..]),
        Some('⏳') => ("\x1b[33m⏳\x1b[0m", &rest['⏳'.len_utf8()..]),
        _ => return format!("\x1b[2m{line}\x1b[0m"),
    };
    format!("{indent}{marker}\x1b[2m{after}\x1b[0m")
}

/// P1-3: opening line of a tool-call block (rendered on stderr during a chat
/// turn, separating tool activity from the streamed reply).
fn tool_block_start(name: &str, args_short: &str, tty: bool) -> String {
    if !tty {
        return format!("┌ tool: {name}{}", opt_args(args_short));
    }
    format!(
        "{} {} {}",
        cyan("┌ tool:", true),
        cyan(name, true),
        dim(opt_args(args_short).as_str(), true)
    )
}

/// P1-3: closing line of a tool-call block.
fn tool_block_done(name: &str, args_short: &str, ok: bool, elapsed_ms: u128, tty: bool) -> String {
    let mark = if ok {
        green("✓", tty)
    } else {
        red("✗", tty)
    };
    if !tty {
        return format!(
            "└ {} {}{} ({}ms)",
            mark,
            name,
            opt_args(args_short),
            elapsed_ms
        );
    }
    format!(
        "{} {} {} {}",
        dim("└", true),
        mark,
        cyan(name, true),
        dim(&format!("({}ms){}", elapsed_ms, opt_args(args_short)), true)
    )
}

fn opt_args(args_short: &str) -> String {
    if args_short.is_empty() {
        String::new()
    } else {
        format!("({args_short})")
    }
}

/// One-line tail preview of accumulated reasoning text (newlines → spaces),
/// capped to `max_chars` with a leading `…`. Used for the in-place CLI status.
fn reasoning_tail(text: &str, max_chars: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| {
            if c.is_control() || c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .collect();
    let trimmed = flat.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= max_chars {
        return trimmed.to_string();
    }
    let tail: String = chars[chars.len() - max_chars..].iter().collect();
    format!("…{tail}")
}

/// Lightweight Markdown → terminal renderer (ANSI). Best-effort: code blocks
/// (indented, dim), inline code (dim), bold, emphasis, headings (bold cyan),
/// list bullets, rules. Falls back to plain text when stdout is not a TTY.
fn render_markdown(text: &str, tty: bool) -> String {
    if !tty || text.trim().is_empty() {
        return text.to_string();
    }
    let mut out = String::new();
    let mut in_code = false;
    let mut code_buf = String::new();
    let mut list_depth: usize = 0;
    for event in MdParser::new(text) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                in_code = true;
                code_buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                out.push_str("\x1b[2m");
                for line in code_buf.trim_end_matches('\n').split('\n') {
                    out.push_str("  ");
                    out.push_str(line);
                    out.push('\n');
                }
                out.push_str("\x1b[0m");
            }
            Event::Text(t) if in_code => code_buf.push_str(&t),
            Event::Start(Tag::Heading { .. }) => out.push_str("\x1b[1;36m"),
            Event::End(TagEnd::Heading(_)) => out.push_str("\x1b[0m\n"),
            Event::End(TagEnd::Paragraph) => out.push('\n'),
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => list_depth = list_depth.saturating_sub(1),
            Event::Start(Tag::Item) => {
                for _ in 0..list_depth.saturating_sub(1) {
                    out.push_str("  ");
                }
                out.push_str("• ");
            }
            Event::End(TagEnd::Item) => out.push('\n'),
            Event::Start(Tag::Strong) => out.push_str("\x1b[1m"),
            Event::End(TagEnd::Strong) => out.push_str("\x1b[22m"),
            Event::Start(Tag::Emphasis) => out.push_str("\x1b[3m"),
            Event::End(TagEnd::Emphasis) => out.push_str("\x1b[23m"),
            Event::Code(c) => out.push_str(&format!("\x1b[2m{c}\x1b[22m")),
            Event::Text(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak => out.push('\n'),
            Event::Rule => out.push_str("\x1b[2m────────\x1b[0m\n"),
            _ => {}
        }
    }
    let trimmed = out.trim_end();
    let mut s = trimmed.to_string();
    s.push('\n');
    s
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

async fn run_web(
    config: Config,
    config_path: String,
    store: Arc<dyn store::Store>,
    attach: bool,
    bind_override: Option<String>,
) -> Result<()> {
    let bind_str = bind_override.unwrap_or_else(|| config.web.bind.clone());
    let bind: std::net::SocketAddr = bind_str.parse().map_err(|e| {
        crate::error::CoworkerError::Config(format!("invalid web bind `{bind_str}`: {e}"))
    })?;

    let (tx, rx) = event_channel();
    let state: SharedState = Arc::new(tokio::sync::RwLock::new(AppState::new(
        config.clone(),
        config_path,
    )));
    hydrate_from_store(&state, store.as_ref()).await?;

    {
        let mut s = state.write().await;
        s.chat_context_visible = true;
        s.push_log("info", format!("WebUI listening on http://{bind}"));
    }

    let auth_token = config.web.effective_auth_token().map(str::to_owned);
    if !bind.ip().is_loopback() && !config.web.auth_enabled() {
        tracing::warn!(
            "web.bind is {bind} without web.auth_token — /api/* and /ws are unauthenticated on the network"
        );
    }
    let engine =
        Arc::new(Engine::new(config, Arc::clone(&store), tx.clone(), Arc::clone(&state)).await);
    spawn_sighup_reload(Arc::clone(&engine));
    engine.clone().spawn_background();
    if attach {
        let mut s = state.write().await;
        s.attach_mode = true;
        s.push_log(
            "info",
            "attach mode — scheduler disabled (shared store with daemon)",
        );
    } else {
        engine.clone().spawn_scheduler();
    }

    web::run(bind, state, engine, store, rx, attach, auth_token).await
}

async fn run_tui(
    config: Config,
    config_path: String,
    store: Arc<dyn store::Store>,
    attach: bool,
) -> Result<()> {
    let (tx, rx) = event_channel();
    let state: SharedState = Arc::new(tokio::sync::RwLock::new(AppState::new(
        config.clone(),
        config_path,
    )));
    hydrate_from_store(&state, store.as_ref()).await?;

    {
        let mut s = state.write().await;
        s.push_log(
            "info",
            format!("unistar-coworker v{} started", env!("CARGO_PKG_VERSION")),
        );
    }

    let engine =
        Arc::new(Engine::new(config, Arc::clone(&store), tx.clone(), Arc::clone(&state)).await);
    spawn_sighup_reload(Arc::clone(&engine));
    engine.clone().spawn_background();
    if attach {
        {
            let mut s = state.write().await;
            s.attach_mode = true;
            s.push_log(
                "info",
                "attach mode — scheduler disabled (shared store with daemon)",
            );
        }
    } else {
        engine.clone().spawn_scheduler();
    }

    let mut terminal = ratatui::init();
    let result = tui::run(&mut terminal, state, engine, store, rx).await;
    ratatui::restore();
    result
}
