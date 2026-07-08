use std::path::PathBuf;
use std::sync::Arc;

use coworker_core::app::{event_channel, AppState, SharedState};
use coworker_core::config::Config;
use coworker_core::engine::Engine;
use coworker_core::error::Result;
use coworker_core::store;

use super::runtime::spawn_sighup_reload;

pub(crate) async fn run_daemon(
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
