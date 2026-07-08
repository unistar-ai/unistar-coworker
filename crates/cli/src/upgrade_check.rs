//! Thin CLI wrapper around [`coworker_core::upgrade`].

use coworker_core::error::Result;

use super::terminal::{emit_json, hint_prefix, use_color_stdout, warn_prefix, yellow};

pub(crate) async fn run_upgrade_check(json: bool) -> Result<()> {
    let info = coworker_core::upgrade::check_upgrade(env!("CARGO_PKG_VERSION")).await;
    if json {
        emit_json(serde_json::to_value(&info).unwrap_or_default());
    } else {
        print_human_report(&info);
    }
    Ok(())
}

fn print_human_report(report: &coworker_core::upgrade::UpgradeInfo) {
    let tty = use_color_stdout();
    println!("current version: {}", report.current);
    if let Some(latest) = &report.latest {
        println!("latest release:  {latest}");
        if report.update_available {
            println!("{} update available", yellow("●", tty));
            if let Some(url) = &report.release_url {
                println!("  release: {url}");
            }
        } else {
            println!("up to date");
        }
    }
    if let Some(w) = &report.warning {
        eprintln!("{} {w}", warn_prefix());
        eprintln!("  {} offline or rate-limited checks exit 0", hint_prefix());
    }
}
