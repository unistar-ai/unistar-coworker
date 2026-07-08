use std::sync::Arc;

use coworker_core::app::{event_channel, AppEvent, AppState, SharedState};
use coworker_core::config::Config;
use coworker_core::engine::Engine;
use coworker_core::error::Result;
use coworker_core::store;

pub(crate) async fn run_headless(
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
