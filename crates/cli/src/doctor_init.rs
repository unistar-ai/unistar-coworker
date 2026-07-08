use std::path::PathBuf;

use coworker_core::diagnostics;
use coworker_core::error::Result;
use coworker_core::exit_codes;

use super::terminal::{bold, emit_json, green, hint_prefix, red, use_color_stdout, yellow};

pub(crate) async fn run_doctor(config_override: Option<PathBuf>, json: bool) -> Result<()> {
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

pub(crate) async fn run_init(
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

    let template = include_str!("../../../coworker.example.yaml");
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
