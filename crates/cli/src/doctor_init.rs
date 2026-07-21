use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use coworker_core::diagnostics::{self, DoctorExtras, DoctorReport};
use coworker_core::error::{CoworkerError, Result};
use coworker_core::exit_codes;

use super::terminal::{bold, emit_json, green, hint_prefix, red, use_color_stdout, yellow};

pub(crate) async fn run_doctor(
    config_override: Option<PathBuf>,
    json: bool,
    bundle: Option<PathBuf>,
) -> Result<()> {
    let (web_status, web_detail) = coworker_web::web_ui_doctor_status();
    let extras = DoctorExtras {
        web_ui_status: Some(web_status),
        web_ui_detail: Some(web_detail),
    };
    let report = diagnostics::run_checks_with_extras(config_override.clone(), extras).await;

    if let Some(path) = bundle {
        write_bundle(&path, &report, config_override.as_ref())?;
        if !json {
            let tty = use_color_stdout();
            println!(
                "{} diagnostic bundle -> {}",
                green("◆", tty),
                path.display()
            );
        }
    }

    print_report(&report, json)?;

    if report.has_failures() {
        std::process::exit(exit_codes::EXIT_CONFIG);
    }
    Ok(())
}

fn print_report(report: &DoctorReport, json: bool) -> Result<()> {
    let tty = use_color_stdout();
    if json {
        emit_json(serde_json::to_value(report).unwrap_or_default());
    } else {
        for c in &report.checks {
            let icon = match c.status {
                "ok" => green("✓", tty),
                "warn" => yellow("⚠", tty),
                _ => red("✗", tty),
            };
            println!("{icon} {:<8} {}", c.name, c.detail);
            if let Some(hint) = &c.hint {
                if c.status == "fail" || c.status == "warn" {
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
    Ok(())
}

fn zip_err(e: impl std::fmt::Display) -> CoworkerError {
    CoworkerError::Workflow(format!("zip: {e}"))
}

fn write_bundle(
    zip_path: &std::path::Path,
    report: &DoctorReport,
    config_override: Option<&PathBuf>,
) -> Result<()> {
    use std::fs::File;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    if let Some(parent) = zip_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let file = File::create(zip_path)?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let doctor_json = serde_json::to_string_pretty(report).map_err(CoworkerError::Json)?;
    zip.start_file("doctor.json", opts).map_err(zip_err)?;
    zip.write_all(doctor_json.as_bytes())?;

    let config_path = resolve_config_path(config_override);
    if let Some(path) = config_path {
        if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            let redacted = diagnostics::redact_coworker_yaml(&raw);
            zip.start_file("coworker.yaml", opts).map_err(zip_err)?;
            zip.write_all(redacted.as_bytes())?;
        }
    }

    let meta = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "rustc": rustc_version(),
    });
    let meta_str = serde_json::to_string_pretty(&meta).map_err(CoworkerError::Json)?;
    zip.start_file("meta.json", opts).map_err(zip_err)?;
    zip.write_all(meta_str.as_bytes())?;

    zip.finish().map_err(zip_err)?;
    Ok(())
}

fn resolve_config_path(config_override: Option<&PathBuf>) -> Option<PathBuf> {
    if let Some(p) = config_override {
        return Some(p.clone());
    }
    [
        PathBuf::from(".coworker/coworker.yaml"),
        PathBuf::from("coworker.yaml"),
    ]
    .into_iter()
    .find(|path| path.exists())
}

fn rustc_version() -> Option<String> {
    std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
}

// ─────────────────────────────────────────────────────────────────────────────
// `init` — create a starter coworker.yaml.
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) async fn run_init(
    force: bool,
    config_override: Option<PathBuf>,
    path: Option<PathBuf>,
    llm_url: Option<String>,
    interactive: bool,
) -> Result<()> {
    let target = path
        .or(config_override)
        .unwrap_or_else(|| PathBuf::from(".coworker/coworker.yaml"));
    if target.exists() && !force {
        eprintln!(
            "{} already exists — use --force to overwrite",
            target.display()
        );
        return Ok(());
    }

    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let use_interactive = interactive && io::stdin().is_terminal() && io::stdout().is_terminal();

    let (llm_url_seed, remote_profile) = if use_interactive {
        run_interactive_prompts().await?
    } else {
        (llm_url, None)
    };

    let template = include_str!("../../../coworker.example.yaml");
    let mut lines: Vec<String> = template.lines().map(String::from).collect();

    if let Some(url) = &llm_url_seed {
        if let Some(idx) = lines.iter().position(|l| {
            let t = l.trim_start();
            t.starts_with("base_url:") && !t.starts_with('#')
        }) {
            lines[idx] = format!("    base_url: {url}");
        }
    }

    if let Some((name, env_var, base_url, model)) = remote_profile {
        if let Some(idx) = lines.iter().position(|l| l.trim() == "llm:") {
            let insert_at = idx + 1;
            let block = [
                format!("  {name}:"),
                format!("    base_url: {base_url}"),
                format!("    model: {model}"),
                "    context_limit: 128000".into(),
                format!("    api_key: ${{{env_var}}}"),
            ];
            for (i, line) in block.into_iter().enumerate() {
                lines.insert(insert_at + i, line);
            }
            lines.insert(idx, format!("llm_profile: {name}"));
            lines.insert(idx, String::new());
        }
    }

    std::fs::write(&target, lines.join("\n"))?;
    let tty = use_color_stdout();
    println!("{} created {}", green("◆", tty), target.display());

    if use_interactive {
        eprintln!("  {} running doctor summary…", hint_prefix());
        let report =
            diagnostics::run_checks_with_extras(Some(target.clone()), DoctorExtras::default())
                .await;
        print_report(&report, false)?;
        if report.has_failures() {
            eprintln!(
                "  {} fix warnings above, then run `unistar-coworker serve`",
                hint_prefix()
            );
        }
    } else {
        eprintln!(
            "  {} edit `llm.model` (25B+ recommended, e.g. gemma4:26b-a4b or qwen3.6:27b) and `chat.workspace`, then run `unistar-coworker doctor`",
            hint_prefix()
        );
    }
    Ok(())
}

async fn run_interactive_prompts(
) -> Result<(Option<String>, Option<(String, String, String, String)>)> {
    println!("unistar-coworker init (interactive)");
    println!();

    let ollama_url = probe_ollama().await;
    let llm_url = if ollama_url {
        println!(
            "{} Ollama detected at http://127.0.0.1:11434 — using http://127.0.0.1:11434/v1",
            green("✓", use_color_stdout())
        );
        print_reference_model_hint();
        Some("http://127.0.0.1:11434/v1".into())
    } else {
        println!(
            "{} Ollama not detected — you can set llm.base_url manually later",
            yellow("!", use_color_stdout())
        );
        None
    };

    let remote_profile = prompt_remote_profile()?;

    Ok((llm_url, remote_profile))
}

fn print_reference_model_hint() {
    println!();
    println!("  Reference-tier models (25B+):");
    println!("    ollama pull gemma4:26b-a4b-it-qat");
    println!("    ollama pull qwen3.6:27b");
    println!("  Set llm.model in coworker.yaml after init (see docs/local-models.md).");
    println!();
}

async fn probe_ollama() -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    else {
        return false;
    };
    client
        .get("http://127.0.0.1:11434/api/tags")
        .send()
        .await
        .ok()
        .is_some_and(|r| r.status().is_success())
}

fn prompt_remote_profile() -> Result<Option<(String, String, String, String)>> {
    print!("Add remote LLM profile? [y/N]: ");
    io::stdout().flush().ok();
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    if !matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        return Ok(None);
    }

    print!("Profile name (e.g. deepseek): ");
    io::stdout().flush().ok();
    line.clear();
    io::stdin().read_line(&mut line)?;
    let name = line.trim().to_string();
    if name.is_empty() {
        return Ok(None);
    }

    print!("API key env var name (e.g. DEEPSEEK_API_KEY): ");
    io::stdout().flush().ok();
    line.clear();
    io::stdin().read_line(&mut line)?;
    let env_var = line.trim().to_string();
    if env_var.is_empty()
        || !env_var
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        eprintln!(
            "{} invalid env var name — skipping remote profile",
            hint_prefix()
        );
        return Ok(None);
    }

    print!("base_url [https://api.deepseek.com/v1]: ");
    io::stdout().flush().ok();
    line.clear();
    io::stdin().read_line(&mut line)?;
    let base_url = line.trim();
    let base_url = if base_url.is_empty() {
        "https://api.deepseek.com/v1".to_string()
    } else {
        base_url.to_string()
    };

    print!("model [deepseek-chat]: ");
    io::stdout().flush().ok();
    line.clear();
    io::stdin().read_line(&mut line)?;
    let model = line.trim();
    let model = if model.is_empty() {
        "deepseek-chat".to_string()
    } else {
        model.to_string()
    };

    Ok(Some((name, env_var, base_url, model)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn init_writes_workspace_first_template() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("coworker.yaml");
        run_init(false, None, Some(path.clone()), None, false)
            .await
            .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("tool_mode: auto"));
    }

    #[tokio::test]
    async fn init_non_interactive_seeds_llm_url() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("coworker.yaml");
        run_init(
            false,
            None,
            Some(path.clone()),
            Some("http://127.0.0.1:11434/v1".into()),
            false,
        )
        .await
        .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("http://127.0.0.1:11434/v1"));
    }

    #[tokio::test]
    async fn init_skips_existing_without_force() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("coworker.yaml");
        std::fs::write(&path, "existing").unwrap();
        run_init(false, None, Some(path.clone()), None, false)
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing");
    }

    #[tokio::test]
    async fn init_interactive_without_tty_uses_cli_llm_url() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("coworker.yaml");
        run_init(
            false,
            None,
            Some(path.clone()),
            Some("http://127.0.0.1:11434/v1".into()),
            true,
        )
        .await
        .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("http://127.0.0.1:11434/v1"));
    }
}
