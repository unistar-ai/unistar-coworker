use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "unistar-coworker",
    about = "Local-first general agent for local LLMs",
    after_help = "EXAMPLES:\n    unistar-coworker tui                                  Terminal UI (default)\n    unistar-coworker serve                            Web UI server\n    unistar-coworker chat                             interactive chat REPL\n    unistar-coworker chat --once \"summarize PR 123\" --json\n    unistar-coworker report ci --repo owner/name\n    unistar-coworker store compact --dry-run --audit-days 30\n\nGlobal flags (--config / -v / -q) go before the subcommand."
)]
pub(crate) struct Cli {
    /// Override config file path (skips discover in .coworker/ / cwd)
    #[arg(long, global = true)]
    pub(crate) config: Option<PathBuf>,
    /// Increase log verbosity (-v = debug, -vv = trace)
    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)]
    pub(crate) verbose: u8,
    /// Decrease log verbosity to warn
    #[arg(short = 'q', long, global = true)]
    pub(crate) quiet: bool,
    /// Disable all ANSI color / box-drawing in output (plain text)
    #[arg(long, global = true)]
    pub(crate) plain: bool,
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Export store reports without chat
    Report {
        #[command(subcommand)]
        kind: ReportKind,
    },
    /// Terminal UI
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
        /// Export a diagnostic zip (doctor json, redacted config, meta)
        #[arg(long, value_name = "PATH")]
        bundle: Option<PathBuf>,
    },
    /// Create a starter coworker.yaml (does not overwrite unless --force)
    Init {
        /// Overwrite an existing coworker.yaml
        #[arg(long)]
        force: bool,
        /// Target path (defaults to ./coworker.yaml)
        #[arg(long)]
        path: Option<PathBuf>,
        /// LLM base_url to seed (e.g. http://localhost:11434/v1)
        #[arg(long)]
        llm_url: Option<String>,
        /// Guided setup when stdin/stdout are a TTY
        #[arg(long)]
        interactive: bool,
    },
    /// Check GitHub Releases for a newer version
    UpgradeCheck {
        /// Emit machine-readable JSON on stdout
        #[arg(long)]
        json: bool,
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
pub(crate) enum ExportTarget {
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
pub(crate) enum ExportFormat {
    Jsonl,
    Html,
}

#[derive(Subcommand)]
pub(crate) enum CatalogCmd {
    /// Print name, path, description
    List {
        /// Emit machine-readable JSON on stdout
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum StoreCommands {
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
    /// Prune old audit entries and legacy store artifacts
    Compact {
        /// Prune audit entries older than N days
        #[arg(long, default_value_t = 90)]
        audit_days: u32,
        /// Preview what would be pruned without deleting anything
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ReportKind {
    /// CI efficiency report (requires GitHub harness)
    Ci {
        /// Repository slug(s) owner/name (repeat flag for multiple)
        #[arg(long = "repo", value_delimiter = ',')]
        repo: Vec<String>,
        #[arg(long, default_value_t = 7)]
        since_days: u32,
        /// Wrap the report in a JSON object on stdout
        #[arg(long)]
        json: bool,
    },
}

use std::sync::Arc;

use clap::CommandFactory;

use coworker_core::config::Config;
use coworker_core::error::Result;
use coworker_core::logging;
use coworker_core::store::open_store;

use super::terminal::set_plain;
use super::{
    catalog, chat, doctor_init, export, report, rpc, runtime, store, upgrade_check,
};

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    set_plain(cli.plain);
    let tui_mode = matches!(cli.command, None | Some(Commands::Tui));
    let chat_repl = matches!(
        cli.command,
        Some(Commands::Chat {
            once: None,
            list_sessions: false,
            ..
        })
    );
    logging::init_tracing(tui_mode, cli.verbose, cli.quiet, chat_repl);

    if let Some(Commands::Completions { shell }) = &cli.command {
        let mut cmd = Cli::command();
        clap_complete::generate(*shell, &mut cmd, "unistar-coworker", &mut std::io::stdout());
        return Ok(());
    }
    if let Some(Commands::Doctor { json, bundle }) = &cli.command {
        return doctor_init::run_doctor(cli.config.clone(), *json, bundle.clone()).await;
    }
    if let Some(Commands::UpgradeCheck { json }) = &cli.command {
        return upgrade_check::run_upgrade_check(*json).await;
    }
    if let Some(Commands::Init {
        force,
        path,
        llm_url,
        interactive,
    }) = &cli.command
    {
        return doctor_init::run_init(
            *force,
            cli.config.clone(),
            path.clone(),
            llm_url.clone(),
            *interactive,
        )
        .await;
    }

    let (config, config_path) = match &cli.config {
        Some(path) => (Config::load(path)?, path.clone()),
        None => Config::discover()?,
    };
    let config_path = config_path.display().to_string();
    let store: Arc<dyn coworker_core::store::Store> = Arc::from(open_store(&config)?);

    match cli.command {
        Some(Commands::Report { kind }) => {
            report::run_report(&config, store.as_ref(), kind).await?;
        }
        Some(Commands::Tui) | None => {
            runtime::run_tui(config, config_path, store).await?;
        }
        Some(Commands::Serve { bind }) => {
            runtime::run_web(config, config_path, store, bind).await?;
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
                chat::list_chat_sessions(store.as_ref(), json, limit).await?;
            } else {
                chat::run_chat_cli(config, store, once, session, json, title, yes, timeout).await?;
            }
        }
        Some(Commands::Store { cmd }) => {
            store::run_store_cmd(config, cmd).await?;
        }
        Some(Commands::Export { target }) => {
            export::run_export_cmd(store.as_ref(), target).await?;
        }
        Some(Commands::Rpc {
            session,
            yes,
            timeout,
        }) => {
            rpc::run_rpc(config, store, session, yes, timeout).await?;
        }
        Some(Commands::Skills { cmd }) => {
            catalog::run_catalog_list("skills", "SKILL.md", cmd).await?;
        }
        Some(Commands::Doctor { .. })
        | Some(Commands::Init { .. })
        | Some(Commands::UpgradeCheck { .. })
        | Some(Commands::Completions { .. }) => {
            unreachable!()
        }
    }
    Ok(())
}
