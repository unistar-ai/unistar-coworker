//! LLM-reviewed Python snippets for chat (`python_run`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::time;

use crate::agent::bash_tool::{
    bash_review_response_schema, parse_bash_review_response_for_tool, BashCommandReview,
    REVIEW_JSON_RETRY_SUFFIX,
};
use crate::agent::context::truncate_chars;
use crate::agent::harness_errors::{
    python_preflight_envelope, python_validation_envelope, ErrorEnvelope,
};
use crate::agent::review_gate::ReviewGateOutcome;
use crate::config::PythonToolConfig;
use crate::error::{CoworkerError, Result};
use crate::llm::LlmClient;

pub const PYTHON_RUN_TOOL: &str = "python_run";

const PYTHON_REVIEW_PROMPT: &str = include_str!("../../prompts/python-review.md");
const PYTHON_REVIEW_MAX_TOKENS: u32 = 1024;

pub fn is_python_tool(name: &str) -> bool {
    name == PYTHON_RUN_TOOL
}

pub fn output_indicates_failure(output: &str) -> bool {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("exit: ") {
            let code = rest.split_whitespace().next().unwrap_or("?");
            return code != "0";
        }
    }
    true
}

pub async fn execute_python_tool(
    config: &PythonToolConfig,
    llm: &LlmClient,
    workspace: &Path,
    args: &Value,
) -> Result<ReviewGateOutcome> {
    let code = args
        .get("code")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CoworkerError::Workflow(
                python_validation_envelope("python_run needs non-empty code", None)
                    .format_tool_error_body(),
            )
        })?;
    let cwd = args.get("cwd").and_then(|v| v.as_str());

    validate_code(code).map_err(|e| match e {
        CoworkerError::Workflow(msg) => CoworkerError::Workflow(
            python_validation_envelope(&msg, Some(code)).format_tool_error_body(),
        ),
        other => other,
    })?;
    if let Some(env) = python_preflight_envelope(code) {
        return Err(CoworkerError::Workflow(env.format_tool_error_body()));
    }

    let review = review_code(llm, code).await?;
    if !review.is_approved() {
        return Ok(ReviewGateOutcome::LlmRejected(review));
    }

    Ok(ReviewGateOutcome::Executed(
        run_python_code(config, workspace, code, cwd, &review.reason_code).await?,
    ))
}

/// Run after human approval — skips LLM review.
pub async fn execute_python_approved(
    config: &PythonToolConfig,
    workspace: &Path,
    args: &Value,
) -> Result<String> {
    let code = args
        .get("code")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CoworkerError::Workflow(
                python_validation_envelope("python_run needs non-empty code", None)
                    .format_tool_error_body(),
            )
        })?;
    let cwd = args.get("cwd").and_then(|v| v.as_str());
    validate_code(code).map_err(|e| match e {
        CoworkerError::Workflow(msg) => CoworkerError::Workflow(
            python_validation_envelope(&msg, Some(code)).format_tool_error_body(),
        ),
        other => other,
    })?;
    let mut out = run_python_code(config, workspace, code, cwd, "HUMAN_APPROVE").await?;
    if !out.contains("review: APPROVE") {
        out = out.replace("review: APPROVE", "review: HUMAN_APPROVE");
    }
    Ok(out)
}

async fn run_python_code(
    config: &PythonToolConfig,
    workspace: &Path,
    code: &str,
    cwd: Option<&str>,
    review_reason: &str,
) -> Result<String> {
    let workdir = resolve_cwd(workspace, cwd).map_err(|e| match e {
        CoworkerError::Workflow(msg) => CoworkerError::Workflow(
            python_validation_envelope(&msg, Some(code)).format_tool_error_body(),
        ),
        other => other,
    })?;

    let started = std::time::Instant::now();
    let output = run_with_timeout(&config.command, code, &workdir, config.timeout_secs).await?;
    let elapsed_ms = started.elapsed().as_millis();
    let review = BashCommandReview {
        verdict: "APPROVE".into(),
        reason_code: review_reason.into(),
        critical_issues: vec![],
        suggestions: vec![],
    };

    Ok(format_output(
        &workdir,
        &review,
        output,
        elapsed_ms,
        config.max_output_chars,
    ))
}

async fn review_code(llm: &LlmClient, code: &str) -> Result<BashCommandReview> {
    let schema = bash_review_response_schema();
    let raw = llm
        .review_python_code_json(
            PYTHON_REVIEW_PROMPT,
            code,
            &schema,
            PYTHON_REVIEW_MAX_TOKENS,
        )
        .await?;
    if let Ok(review) = parse_bash_review_response_for_tool(&raw, PYTHON_RUN_TOOL) {
        return Ok(review);
    }
    tracing::warn!("python_run review JSON parse failed, retrying with JSON-only nudge");
    let retry_prompt = format!("{PYTHON_REVIEW_PROMPT}{REVIEW_JSON_RETRY_SUFFIX}");
    let raw = llm
        .review_python_code_json(&retry_prompt, code, &schema, PYTHON_REVIEW_MAX_TOKENS)
        .await?;
    parse_bash_review_response_for_tool(&raw, PYTHON_RUN_TOOL)
}

fn validate_code(code: &str) -> Result<()> {
    if code.is_empty() {
        return Err(CoworkerError::Workflow("python_run code is empty".into()));
    }
    if code.contains('\0') {
        return Err(CoworkerError::Workflow(
            "python_run code must not contain null bytes".into(),
        ));
    }
    Ok(())
}

fn resolve_cwd(workspace: &Path, cwd: Option<&str>) -> Result<PathBuf> {
    let base = workspace.canonicalize().map_err(|e| {
        CoworkerError::Workflow(format!(
            "python_run workspace {:?} is not a directory: {e}",
            workspace.display()
        ))
    })?;
    if !base.is_dir() {
        return Err(CoworkerError::Workflow(format!(
            "python_run workspace {:?} is not a directory",
            workspace.display()
        )));
    }
    let path = match cwd {
        None | Some("") => base,
        Some(raw) => {
            if raw.contains("..") {
                return Err(CoworkerError::Workflow(
                    "python_run cwd must not contain '..'".into(),
                ));
            }
            let path = PathBuf::from(raw);
            if path.is_absolute() {
                path
            } else {
                base.join(path)
            }
        }
    };
    let canonical = path.canonicalize().map_err(|e| {
        CoworkerError::Workflow(format!(
            "python_run cwd {:?} is not a directory: {e}",
            cwd.unwrap_or(".")
        ))
    })?;
    if !canonical.is_dir() {
        return Err(CoworkerError::Workflow(format!(
            "python_run cwd {:?} is not a directory",
            cwd.unwrap_or(".")
        )));
    }
    Ok(canonical)
}

struct CommandOutput {
    status_code: Option<i32>,
    stdout: String,
    stderr: String,
}

async fn run_with_timeout(
    python: &str,
    code: &str,
    cwd: &Path,
    timeout_secs: u64,
) -> Result<CommandOutput> {
    let timeout = Duration::from_secs(timeout_secs.clamp(1, 300));
    let python = python.trim();
    if python.is_empty() {
        return Err(CoworkerError::Workflow(
            python_validation_envelope("python_run: configured python command is empty", None)
                .format_tool_error_body(),
        ));
    }
    let code = code.to_string();
    let cwd = cwd.to_path_buf();
    let fut = async move {
        let mut child = Command::new(python)
            .arg("-u")
            .arg("-")
            .current_dir(&cwd)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                CoworkerError::Workflow(
                    python_validation_envelope(
                        &format!("python_run spawn failed ({python}): {e}"),
                        None,
                    )
                    .format_tool_error_body(),
                )
            })?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(code.as_bytes()).await.map_err(|e| {
                CoworkerError::Workflow(format!("python_run stdin write failed: {e}"))
            })?;
        }

        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let stdout_task = async {
            let mut buf = String::new();
            if let Some(mut pipe) = stdout.take() {
                pipe.read_to_string(&mut buf).await.ok();
            }
            buf
        };
        let stderr_task = async {
            let mut buf = String::new();
            if let Some(mut pipe) = stderr.take() {
                pipe.read_to_string(&mut buf).await.ok();
            }
            buf
        };
        let (out, err) = tokio::join!(stdout_task, stderr_task);
        let status = child
            .wait()
            .await
            .map_err(|e| CoworkerError::Workflow(format!("python_run wait failed: {e}")))?;
        Ok(CommandOutput {
            status_code: status.code(),
            stdout: out,
            stderr: err,
        })
    };
    time::timeout(timeout, fut).await.map_err(|_| {
        CoworkerError::Workflow(
            ErrorEnvelope {
                code: "PYTHON_TIMEOUT".into(),
                tool_name: PYTHON_RUN_TOOL.into(),
                what: "Python script timed out".into(),
                why: format!("Exceeded {timeout_secs}s"),
                try_steps: vec![
                    "Increase chat.python.timeout_secs".into(),
                    "Simplify the script or process less data".into(),
                ],
                example: None,
                detail: None,
            }
            .format_tool_error_body(),
        )
    })?
}

fn format_output(
    cwd: &Path,
    review: &BashCommandReview,
    output: CommandOutput,
    elapsed_ms: u128,
    max_chars: usize,
) -> String {
    let code = output
        .status_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".into());
    let mut body = format!(
        "python_run\nreview: APPROVE ({reason})\ncwd: {}\nexit: {code} ({elapsed_ms}ms)\n",
        cwd.display(),
        reason = review.reason_code
    );
    let stdout = crate::terminal::sanitize_terminal_output(&output.stdout);
    let stderr = crate::terminal::sanitize_terminal_output(&output.stderr);
    if !stdout.trim().is_empty() {
        body.push_str("\nstdout:\n");
        body.push_str(&truncate_chars(stdout.trim_end(), max_chars));
    }
    if !stderr.trim().is_empty() {
        body.push_str("\n\nstderr:\n");
        body.push_str(&truncate_chars(stderr.trim_end(), max_chars));
    }
    if stdout.trim().is_empty() && stderr.trim().is_empty() {
        body.push_str("\n(no output)");
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::bash_tool::BashCriticalIssue;
    use crate::agent::harness_errors::python_safety_reject_envelope;

    #[test]
    fn output_indicates_failure_reads_exit_code() {
        assert!(!output_indicates_failure("python_run\nexit: 0 (1ms)\n"));
        assert!(output_indicates_failure("python_run\nexit: 1 (1ms)\n"));
    }

    #[test]
    fn validate_rejects_null_bytes() {
        assert!(validate_code("print('hi')").is_ok());
        assert!(validate_code("a\0b").is_err());
    }

    #[test]
    fn preflight_blocks_os_system() {
        assert!(python_preflight_envelope("import os\nos.system('rm -rf /')").is_some());
        assert!(python_preflight_envelope("print(1+1)").is_none());
    }

    #[test]
    fn parse_review_accepts_plain_json() {
        let raw = r#"{"verdict":"APPROVE","reason_code":"SUCCESS","critical_issues":[],"suggestions":[]}"#;
        let review = parse_bash_review_response_for_tool(raw, PYTHON_RUN_TOOL).unwrap();
        assert!(review.is_approved());
    }

    #[test]
    fn format_rejection_surfaces_issues() {
        let review = BashCommandReview {
            verdict: "REJECT".into(),
            reason_code: "RISK_FOUND".into(),
            critical_issues: vec![BashCriticalIssue {
                line_number: 2,
                code_snippet: "os.system('rm -rf /')".into(),
                risk_type: "HIGH_RISK_COMMAND".into(),
                description: "shell via os.system".into(),
            }],
            suggestions: vec!["print('use pathlib only')".into()],
        };
        let msg = python_safety_reject_envelope("import os\nos.system('x')", &review)
            .format_harness_nudge();
        assert!(msg.contains("[Harness]"));
        assert!(msg.contains("shell via os.system"));
    }

    #[test]
    fn builtin_review_prompt_is_non_empty() {
        let prompt = include_str!("../../prompts/python-review.md");
        assert!(prompt.contains("python_run"));
        assert!(prompt.contains("待审查"));
    }
}
