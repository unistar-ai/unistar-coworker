mod agent;
mod app;
mod approval_payload;
mod config;
mod engine;
mod error;
mod github;
mod llm;
mod logging;
mod mcp;
mod output;
mod store;
mod terminal;
mod tui;
mod web;

use std::sync::Arc;

use clap::{Parser, Subcommand};

use app::{event_channel, hydrate_from_store, AppEvent, AppState, SharedState};
use config::Config;
use engine::Engine;
use error::Result;
use store::open_store;

#[derive(Parser)]
#[command(
    name = "unistar-coworker",
    about = "Local GitHub ops secretary with TUI"
)]
struct Cli {
    /// Attach to daemon store only — do not start a local cron scheduler
    #[arg(long)]
    attach: bool,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a workflow once without TUI (default: daily-work)
    RunOnce {
        #[arg(long, default_value = "daily-work")]
        workflow: String,
    },
    /// Export store reports without running a full workflow
    Report {
        #[command(subcommand)]
        kind: ReportKind,
    },
    /// Debug triage for a single PR (stub in v0.1)
    TriagePr {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        pr: u32,
    },
    /// Headless daemon: cron scheduler without TUI (Phase 2)
    Daemon,
    /// Web UI server (browser)
    Serve {
        /// Override config `web.bind` (e.g. 127.0.0.1:8787)
        #[arg(long)]
        bind: Option<String>,
    },
    /// Interactive chat REPL (Phase 2+)
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
}

#[derive(Subcommand)]
enum CatalogCmd {
    /// Print name, path, description
    List,
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
    },
}

#[derive(Subcommand)]
enum ReportKind {
    /// On-call handoff pack from local store (no MCP)
    Oncall,
    /// CI efficiency report (requires MCP)
    Ci {
        #[arg(long, default_value_t = 7)]
        since_days: u32,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    logging::init_tracing(cli.command.is_none());

    let (config, config_path) = Config::discover()?;
    let config_path = config_path.display().to_string();
    let store = Arc::from(open_store(&config)?);

    match cli.command {
        Some(Commands::RunOnce { workflow }) => {
            run_headless(config, store, &workflow).await?;
        }
        Some(Commands::Report { kind }) => {
            run_report(&config, store.as_ref(), kind).await?;
        }
        Some(Commands::TriagePr { repo, pr }) => {
            run_triage_pr(config, store, &repo, pr).await?;
        }
        Some(Commands::Daemon) => {
            run_daemon(config, store).await?;
        }
        Some(Commands::Serve { bind }) => {
            run_web(config, config_path, store, cli.attach, bind).await?;
        }
        Some(Commands::Chat {
            once,
            session,
            list_sessions,
        }) => {
            if list_sessions {
                list_chat_sessions(store.as_ref()).await?;
            } else {
                run_chat_cli(config, store, once, session).await?;
            }
        }
        Some(Commands::Store { cmd }) => {
            run_store_cmd(config, cmd).await?;
        }
        Some(Commands::Workflows { cmd }) => {
            run_workflows_list(cmd).await?;
        }
        Some(Commands::Skills { cmd }) => {
            run_catalog_list("skills", "SKILL.md", cmd).await?;
        }
        None => {
            run_tui(config, config_path, store, cli.attach).await?;
        }
    }
    Ok(())
}

async fn run_report(config: &Config, store: &dyn store::Store, kind: ReportKind) -> Result<()> {
    use agent::oncall::build_handoff_markdown;

    match kind {
        ReportKind::Oncall => {
            let md = build_handoff_markdown(store).await?;
            println!("{md}");
        }
        ReportKind::Ci { since_days: _ } => {
            let github = github::spawn_github(config).await;
            let md =
                agent::ci_efficiency::build_ci_efficiency_markdown(config, github.as_ref()).await?;
            print!("{md}");
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
    use store::{compact, format_compact_summary, format_migrate_summary, migrate, CompactOptions};

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
            println!("{}", format_migrate_summary(&stats));
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
        } => {
            let opts = CompactOptions {
                audit_days,
                digest_keep,
                workflow_runs_days,
            };
            let path = config.storage_path();
            let stats = compact(
                config.storage.backend,
                path.clone(),
                config.storage.wal,
                &opts,
            )?;
            println!("{}", format_compact_summary(&stats));
            eprintln!(
                "compacted {:?} store at {}",
                config.storage.backend,
                path.display()
            );
        }
    }
    Ok(())
}

async fn run_workflows_list(cmd: CatalogCmd) -> Result<()> {
    use engine::WORKFLOWS;

    match cmd {
        CatalogCmd::List => {
            for wf in WORKFLOWS {
                let skills = if wf.default_skills.is_empty() {
                    "—".into()
                } else {
                    wf.default_skills.join(", ")
                };
                println!("{}\t{}\tskills: {skills}", wf.id, wf.description);
            }
        }
    }
    Ok(())
}

async fn run_catalog_list(root: &str, leaf: &str, cmd: CatalogCmd) -> Result<()> {
    use engine::{load_markdown_spec, load_skill_with_base};
    use std::path::Path;

    match cmd {
        CatalogCmd::List => {
            let root_path = Path::new(root);
            if !root_path.is_dir() {
                println!("(no {root}/ directory)");
                return Ok(());
            }
            let mut entries: Vec<_> = std::fs::read_dir(root_path)?
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .collect();
            entries.sort_by_key(|e| e.file_name());
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
                        println!("{title}\t{}\t{desc}", path.display());
                        if !spec.skill_refs.is_empty() {
                            println!("  skills: {}", spec.skill_refs.join(", "));
                        }
                    }
                    Err(e) => {
                        eprintln!("{}: {e}", path.display());
                    }
                }
            }
        }
    }
    Ok(())
}

async fn run_triage_pr(
    config: Config,
    store: Arc<dyn store::Store>,
    repo: &str,
    pr_number: u32,
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

    let outcome = triage_pr(
        &config,
        github.as_ref(),
        &llm,
        store.as_ref(),
        &classify_skills,
        repo,
        &pr_line,
        None,
    )
    .await?;

    println!("# Triage {repo}#{pr_number}\n");
    for line in outcome.preamble {
        println!("{line}");
    }
    for run in &outcome.runs {
        println!("\n## {:?}\n", run.verdict);
        for line in &run.lines {
            println!("{line}");
        }
    }
    Ok(())
}

async fn run_headless(config: Config, store: Arc<dyn store::Store>, workflow: &str) -> Result<()> {
    let (tx, mut rx) = event_channel();
    let state: SharedState = Arc::new(tokio::sync::RwLock::new(AppState::new(
        config.clone(),
        "headless".into(),
    )));
    let engine = Arc::new(Engine::new(config, Arc::clone(&store), tx, Arc::clone(&state)).await);

    let printer = tokio::spawn(async move {
        while let Ok(ev) = rx.recv().await {
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
    println!("{msg}");
    Ok(())
}

async fn run_daemon(config: Config, store: Arc<dyn store::Store>) -> Result<()> {
    let (tx, _rx) = event_channel();
    let state: SharedState = Arc::new(tokio::sync::RwLock::new(AppState::new(
        config.clone(),
        "daemon".into(),
    )));
    let engine = Arc::new(Engine::new(config, Arc::clone(&store), tx, Arc::clone(&state)).await);
    engine.clone().spawn_background();
    engine.clone().spawn_scheduler();

    eprintln!("unistar-coworker daemon started (cron scheduler active; Ctrl-C to stop)");
    tokio::signal::ctrl_c().await?;
    eprintln!("daemon shutting down");
    Ok(())
}

async fn run_chat_cli(
    config: Config,
    store: Arc<dyn store::Store>,
    once: Option<String>,
    session: Option<uuid::Uuid>,
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
    let engine = Arc::new(Engine::new(config, Arc::clone(&store), tx, Arc::clone(&state)).await);

    let mut session_id = session;

    if let Some(msg) = once {
        let result = run_chat_with_progress(&engine, &mut rx, session_id, &msg).await?;
        println!("{}", result.assistant_message);
        for tc in &result.tool_calls {
            eprintln!("[tool {}] {}", tc.tool_name, tc.output);
        }
        return Ok(());
    }

    eprintln!("unistar-coworker chat (type /quit to exit)");
    loop {
        print!("you> ");
        use std::io::Write;
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "/quit" || line == "exit" || line == "quit" {
            break;
        }
        match run_chat_with_progress(&engine, &mut rx, session_id, line).await {
            Ok(result) => {
                session_id = Some(result.session_id);
                println!("\nassistant> {}", result.assistant_message);
                for tc in &result.tool_calls {
                    eprintln!("[tool {}] {}", tc.tool_name, tc.output);
                }
                println!();
            }
            Err(e) => eprintln!("error: {e}\n"),
        }
    }
    Ok(())
}

async fn run_chat_with_progress(
    engine: &Engine,
    rx: &mut tokio::sync::broadcast::Receiver<AppEvent>,
    session_id: Option<uuid::Uuid>,
    message: &str,
) -> Result<agent::chat_loop::ChatTurnResult> {
    let listener = {
        let mut rx = rx.resubscribe();
        tokio::spawn(async move {
            while let Ok(ev) = rx.recv().await {
                match ev {
                    AppEvent::ChatProgress(p) if p.show_in_log() => {
                        eprintln!("{}", p.display_line());
                    }
                    AppEvent::ChatReply => break,
                    _ => {}
                }
            }
        })
    };

    let result = engine.run_chat(session_id, message).await;
    listener.abort();
    while let Ok(ev) = rx.try_recv() {
        if let AppEvent::ChatProgress(p) = ev {
            if p.show_in_log() {
                eprintln!("{}", p.display_line());
            }
        }
    }
    result
}

async fn list_chat_sessions(store: &dyn store::Store) -> Result<()> {
    let sessions = store.list_chat_sessions(20).await?;
    if sessions.is_empty() {
        println!("No chat sessions.");
        return Ok(());
    }
    for s in sessions {
        println!(
            "{}  {}  {}",
            s.id,
            s.created_at.format("%Y-%m-%d %H:%M"),
            s.title
        );
    }
    Ok(())
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
        s.push_log("info", "unistar-coworker v0.3 started");
    }

    let engine =
        Arc::new(Engine::new(config, Arc::clone(&store), tx.clone(), Arc::clone(&state)).await);
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
