use std::sync::Arc;

use coworker_core::config::Config;
use coworker_core::error::Result;
use coworker_core::exit_codes;
use coworker_core::store;

use super::terminal::{
    bold, emit_json, green, hint_prefix, panel, red, timeout_prefix, use_color_stdout, yellow,
};

pub(crate) async fn run_triage_pr(
    config: Config,
    store: Arc<dyn store::Store>,
    repo: &str,
    pr_number: u32,
    json: bool,
    timeout: Option<u64>,
) -> Result<()> {
    use coworker_core::agent::parse::parse_pr_line;
    use coworker_core::agent::triage::triage_pr;
    use coworker_core::engine::load_classify_skills_for_triage;

    let github = coworker_core::github::spawn_github(&config).await;
    let llm_online = coworker_core::llm::ollama::probe(&config.llm).await;
    let llm = coworker_core::llm::LlmClient::new(config.llm.clone(), llm_online);
    let classify_skills = load_classify_skills_for_triage(&[])?;

    let list_text = coworker_core::github::helpers::gh_tool(
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
            coworker_core::error::CoworkerError::Workflow(format!(
                "PR #{pr_number} not found in {repo}"
            ))
        })?;

    let triage_fut = triage_pr(
        &config,
        github.as_ref(),
        &llm,
        store.as_ref(),
        &classify_skills,
        repo,
        &pr_line,
        None,
    );
    let outcome = match timeout {
        Some(secs) => {
            match tokio::time::timeout(std::time::Duration::from_secs(secs), triage_fut).await {
                Ok(r) => r?,
                Err(_) => {
                    if json {
                        emit_json(
                            serde_json::json!({ "ok": false, "repo": repo, "pr": pr_number, "error": "timeout" }),
                        );
                    } else {
                        eprintln!("{} after {secs}s", timeout_prefix());
                        eprintln!(
                            "  {} increase --timeout or check LLM latency",
                            hint_prefix()
                        );
                    }
                    std::process::exit(exit_codes::EXIT_TIMEOUT);
                }
            }
        }
        None => triage_fut.await?,
    };

    if json {
        let runs: Vec<_> = outcome
            .runs
            .iter()
            .map(|r| {
                serde_json::json!({
                    "verdict": format!("{:?}", r.verdict),
                    "lines": r.lines,
                })
            })
            .collect();
        emit_json(serde_json::json!({
            "ok": true,
            "repo": repo,
            "pr": pr_number,
            "preamble": outcome.preamble,
            "fallback_attention": outcome.fallback_attention,
            "runs": runs,
        }));
    } else {
        let tty = use_color_stdout();
        println!(
            "{}",
            panel(
                &format!("◆ Triage {repo}#{pr_number}"),
                &outcome
                    .preamble
                    .iter()
                    .map(|l| l.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                tty
            )
        );
        for run in &outcome.runs {
            let verdict = format!("{:?}", run.verdict);
            let colored = if verdict.to_lowercase().starts_with("pass") {
                green(&verdict, tty)
            } else if verdict.to_lowercase().starts_with("fail") {
                red(&verdict, tty)
            } else {
                yellow(&verdict, tty)
            };
            println!("\n{} {}", bold("verdict:", tty), colored);
            for line in &run.lines {
                println!("{line}");
            }
        }
    }
    Ok(())
}
