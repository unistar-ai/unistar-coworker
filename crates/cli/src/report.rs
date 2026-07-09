use coworker_core::config::Config;
use coworker_core::error::Result;
use coworker_core::exit_codes;
use coworker_core::store;

use super::args::ReportKind;
use super::terminal::{emit_json, err_prefix, render_markdown, use_color_stdout};

pub(crate) async fn run_report(
    config: &Config,
    _store: &dyn store::Store,
    kind: ReportKind,
) -> Result<()> {
    let json = match &kind {
        ReportKind::Ci { json, .. } => *json,
    };
    let result: Result<(&'static str, String, Option<u32>)> = match kind {
        ReportKind::Ci {
            repo,
            since_days,
            json: _,
        } => {
            let github = coworker_core::github::spawn_github(config).await;
            coworker_core::agent::ci_efficiency::build_ci_efficiency_markdown(
                config,
                github.as_ref(),
                &repo,
            )
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
