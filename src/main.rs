mod agent;
mod app;
mod config;
mod engine;
mod error;
mod llm;
mod mcp;
mod output;
mod store;
mod tui;

use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use app::{event_channel, hydrate_from_store, AppState, SharedState};
use config::Config;
use engine::Engine;
use error::Result;
use store::open_store;

#[derive(Parser)]
#[command(name = "unistar-coworker", about = "Local GitHub ops secretary with TUI")]
struct Cli {
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
    /// Debug triage for a single PR (stub in v0.1)
    TriagePr {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        pr: u32,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("unistar_coworker=info".parse().unwrap()))
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let (config, config_path) = Config::discover()?;
    let config_path = config_path.display().to_string();
    let store = Arc::from(open_store(&config)?);

    match cli.command {
        Some(Commands::RunOnce { workflow }) => {
            run_headless(config, store, &workflow).await?;
        }
        Some(Commands::TriagePr { repo, pr }) => {
            tracing::info!("triage {repo}#{pr} — use TUI daily-work for full flow in v0.1");
        }
        None => {
            run_tui(config, config_path, store).await?;
        }
    }
    Ok(())
}

async fn run_headless(config: Config, store: Arc<dyn store::Store>, workflow: &str) -> Result<()> {
    let (tx, _rx) = event_channel();
    let state: SharedState = Arc::new(tokio::sync::RwLock::new(AppState::new(
        config.clone(),
        "headless".into(),
    )));
    let engine = Arc::new(Engine::new(config, Arc::clone(&store), tx, Arc::clone(&state)).await);
    let msg = engine.run_workflow(workflow).await?;
    println!("{msg}");
    Ok(())
}

async fn run_tui(config: Config, config_path: String, store: Arc<dyn store::Store>) -> Result<()> {
    let (tx, rx) = event_channel();
    let state: SharedState = Arc::new(tokio::sync::RwLock::new(AppState::new(
        config.clone(),
        config_path,
    )));
    hydrate_from_store(&state, store.as_ref()).await?;

    {
        let mut s = state.write().await;
        s.push_log("info", "unistar-coworker v0.1 started");
    }

    let engine = Arc::new(Engine::new(
        config,
        Arc::clone(&store),
        tx.clone(),
        Arc::clone(&state),
    )
    .await);
    engine.clone().spawn_background();

    let mut terminal = ratatui::init();
    let result = tui::run(&mut terminal, state, engine, store, rx).await;
    ratatui::restore();
    result
}
