use std::sync::Arc;

use coworker_core::app::{event_channel, hydrate_from_store, AppState, SharedState};
use coworker_core::config::Config;
use coworker_core::engine::Engine;
use coworker_core::error::Result;
use coworker_core::store;

#[cfg(unix)]
pub(crate) fn spawn_sighup_reload(engine: Arc<Engine>) {
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
pub(crate) fn spawn_sighup_reload(_engine: Arc<Engine>) {}

pub(crate) async fn run_web(
    config: Config,
    config_path: String,
    store: Arc<dyn store::Store>,
    bind_override: Option<String>,
) -> Result<()> {
    let bind_str = bind_override.unwrap_or_else(|| config.web.bind.clone());
    let bind: std::net::SocketAddr = bind_str.parse().map_err(|e| {
        coworker_core::error::CoworkerError::Config(format!("invalid web bind `{bind_str}`: {e}"))
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
        s.app_version = env!("CARGO_PKG_VERSION").to_string();
        s.push_log("info", format!("WebUI listening on http://{bind}"));
    }

    coworker_core::app::AppState::spawn_upgrade_check(
        Arc::clone(&state),
        env!("CARGO_PKG_VERSION"),
    );

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

    coworker_web::run(bind, state, engine, store, rx, false, auth_token).await
}

pub(crate) async fn run_tui(
    config: Config,
    config_path: String,
    store: Arc<dyn store::Store>,
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

    let mut terminal = ratatui::init();
    let result = coworker_tui::run(&mut terminal, state, engine, store, rx).await;
    ratatui::restore();
    result
}
