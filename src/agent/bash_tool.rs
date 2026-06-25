//! LLM-reviewed local shell commands for chat (`bash_run`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::time;

use crate::agent::context::truncate_chars;
use crate::agent::harness_errors::{bash_preflight_envelope, bash_validation_envelope};
use crate::agent::review_gate::ReviewGateOutcome;
use crate::config::BashToolConfig;
use crate::error::{CoworkerError, Result};
use crate::llm::LlmClient;

pub const BASH_RUN_TOOL: &str = "bash_run";

const BASH_REVIEW_PROMPT: &str = include_str!("../../prompts/bash-review.md");
const BASH_REVIEW_MAX_TOKENS: u32 = 1024;
const MAX_COMMAND_LINES: usize = 200;
const MAX_COMMAND_CHARS: usize = 32_768;

pub fn is_bash_tool(name: &str) -> bool {
    name == BASH_RUN_TOOL
}

/// Non-zero exit code in formatted `bash_run` output counts as tool failure.
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

#[derive(Debug, Clone, Deserialize)]
pub struct BashCriticalIssue {
    #[serde(default)]
    pub line_number: u32,
    #[serde(default)]
    pub code_snippet: String,
    #[serde(default)]
    pub risk_type: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BashCommandReview {
    pub verdict: String,
    #[serde(default)]
    pub reason_code: String,
    #[serde(default)]
    pub critical_issues: Vec<BashCriticalIssue>,
    #[serde(default)]
    pub suggestions: Vec<String>,
}

impl BashCommandReview {
    pub fn is_approved(&self) -> bool {
        self.verdict.eq_ignore_ascii_case("APPROVE")
    }
}

pub fn bash_review_response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "verdict": {
                "type": "string",
                "enum": ["APPROVE", "REJECT"]
            },
            "reason_code": {
                "type": "string",
                "enum": ["SUCCESS", "RISK_FOUND"]
            },
            "critical_issues": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "line_number": { "type": "integer" },
                        "code_snippet": { "type": "string" },
                        "risk_type": {
                            "type": "string",
                            "enum": [
                                "HIGH_RISK_COMMAND",
                                "AI_HALLUCINATION",
                                "MISSING_ERROR_HANDLING",
                                "SECURITY_VULNERABILITY"
                            ]
                        },
                        "description": { "type": "string" }
                    },
                    "required": ["line_number", "code_snippet", "risk_type", "description"],
                    "additionalProperties": false
                }
            },
            "suggestions": {
                "type": "array",
                "items": { "type": "string" }
            }
        },
        "required": ["verdict", "reason_code", "critical_issues", "suggestions"],
        "additionalProperties": false
    })
}

/// Run `command` after built-in LLM safety review (reject → caller may queue human approval).
pub async fn execute_bash_tool(
    config: &BashToolConfig,
    llm: &LlmClient,
    workspace: &Path,
    args: &Value,
) -> Result<ReviewGateOutcome> {
    let command = extract_command(args)?;
    let cwd = args.get("cwd").and_then(|v| v.as_str());

    validate_command(command).map_err(|e| validation_err(e, Some(command)))?;
    if let Some(env) = bash_preflight_envelope(command) {
        return Err(CoworkerError::Workflow(env.format_tool_error_body()));
    }
    let review = review_command(llm, command).await?;
    if !review.is_approved() {
        return Ok(ReviewGateOutcome::LlmRejected(review));
    }

    Ok(ReviewGateOutcome::Executed(
        run_bash_command(config, workspace, command, cwd, &review.reason_code).await?,
    ))
}

/// Run after human approval — skips LLM review (preflight already passed before queueing).
pub async fn execute_bash_approved(
    config: &BashToolConfig,
    workspace: &Path,
    args: &Value,
) -> Result<String> {
    let command = extract_command(args)?;
    let cwd = args.get("cwd").and_then(|v| v.as_str());
    validate_command(command).map_err(|e| validation_err(e, Some(command)))?;
    let mut out = run_bash_command(config, workspace, command, cwd, "HUMAN_APPROVE").await?;
    if !out.starts_with("review:") {
        out = format!("review: HUMAN_APPROVE\n{out}");
    }
    Ok(out)
}

fn extract_command(args: &Value) -> Result<&str> {
    args.get("command")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CoworkerError::Workflow("bash_run needs non-empty command".into()))
}

fn validation_err(e: CoworkerError, command: Option<&str>) -> CoworkerError {
    match e {
        CoworkerError::Workflow(msg) => CoworkerError::Workflow(
            bash_validation_envelope(&msg, command).format_tool_error_body(),
        ),
        other => other,
    }
}

async fn run_bash_command(
    config: &BashToolConfig,
    workspace: &Path,
    command: &str,
    cwd: Option<&str>,
    review_reason: &str,
) -> Result<String> {
    let workdir =
        resolve_cwd_in_workspace(workspace, cwd).map_err(|e| validation_err(e, Some(command)))?;
    let script_body = normalize_command_text(command);

    let started = std::time::Instant::now();
    let output =
        run_command_with_timeout(&script_body, &workdir, config.timeout_secs, command).await?;
    let elapsed_ms = started.elapsed().as_millis();
    let review = BashCommandReview {
        verdict: "APPROVE".into(),
        reason_code: review_reason.into(),
        critical_issues: vec![],
        suggestions: vec![],
    };
    Ok(format_output(
        command,
        &review,
        &workdir,
        output,
        elapsed_ms,
        config.max_output_chars,
    ))
}

async fn review_command(llm: &LlmClient, command: &str) -> Result<BashCommandReview> {
    let raw = llm
        .review_bash_command_json(
            BASH_REVIEW_PROMPT,
            command,
            &bash_review_response_schema(),
            BASH_REVIEW_MAX_TOKENS,
        )
        .await?;
    parse_bash_review_response(&raw)
}

pub fn parse_bash_review_response(content: &str) -> Result<BashCommandReview> {
    let trimmed = content.trim();
    if let Ok(review) = serde_json::from_str::<BashCommandReview>(trimmed) {
        return Ok(review);
    }
    let unfenced = strip_json_fence(trimmed);
    if unfenced != trimmed {
        if let Ok(review) = serde_json::from_str::<BashCommandReview>(&unfenced) {
            return Ok(review);
        }
    }
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                let slice = &trimmed[start..=end];
                if let Ok(review) = serde_json::from_str::<BashCommandReview>(slice) {
                    return Ok(review);
                }
            }
        }
    }
    Err(CoworkerError::Workflow(
        bash_validation_envelope(
            &format!(
                "bash_run LLM review returned invalid JSON: {}",
                truncate_chars(trimmed, 400)
            ),
            None,
        )
        .format_tool_error_body(),
    ))
}

fn strip_json_fence(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        return rest
            .trim_start_matches('\n')
            .trim_end_matches("```")
            .trim()
            .to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("```") {
        return rest
            .trim_start_matches('\n')
            .trim_end_matches("```")
            .trim()
            .to_string();
    }
    trimmed.to_string()
}

fn validate_command(command: &str) -> Result<()> {
    if command.is_empty() {
        return Err(CoworkerError::Workflow("bash_run command is empty".into()));
    }
    if command.contains('\0') {
        return Err(CoworkerError::Workflow(
            "bash_run command must not contain null bytes".into(),
        ));
    }
    if command.len() > MAX_COMMAND_CHARS {
        return Err(CoworkerError::Workflow(format!(
            "bash_run command exceeds {MAX_COMMAND_CHARS} characters"
        )));
    }
    let line_count = command.lines().count();
    if line_count > MAX_COMMAND_LINES {
        return Err(CoworkerError::Workflow(format!(
            "bash_run command exceeds {MAX_COMMAND_LINES} lines"
        )));
    }
    Ok(())
}

fn normalize_command_text(command: &str) -> String {
    command.replace("\r\n", "\n").replace('\r', "\n")
}

fn resolve_cwd_in_workspace(workspace: &Path, cwd: Option<&str>) -> Result<PathBuf> {
    let base = workspace.canonicalize().map_err(|e| {
        CoworkerError::Workflow(format!(
            "bash_run workspace {:?} is not a directory: {e}",
            workspace.display()
        ))
    })?;
    if !base.is_dir() {
        return Err(CoworkerError::Workflow(format!(
            "bash_run workspace {:?} is not a directory",
            workspace.display()
        )));
    }
    let path = match cwd {
        None | Some("") => base,
        Some(raw) => {
            if raw.contains("..") {
                return Err(CoworkerError::Workflow(
                    "bash_run cwd must not contain '..'".into(),
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
            "bash_run cwd {:?} is not a directory: {e}",
            cwd.unwrap_or(".")
        ))
    })?;
    if !canonical.is_dir() {
        return Err(CoworkerError::Workflow(format!(
            "bash_run cwd {:?} is not a directory",
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

async fn run_command_with_timeout(
    command: &str,
    cwd: &Path,
    timeout_secs: u64,
    command_for_errors: &str,
) -> Result<CommandOutput> {
    let timeout = Duration::from_secs(timeout_secs.clamp(1, 300));
    let is_multiline = command.contains('\n');
    let command = command.to_string();
    let cwd = cwd.to_path_buf();
    let command_for_errors = command_for_errors.to_string();
    let fut = async move {
        let mut child = if is_multiline {
            Command::new("sh")
                .arg("-s")
                .current_dir(&cwd)
                .env("CURL_PROGRESS_BAR", "off")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| CoworkerError::Workflow(format!("bash_run spawn failed: {e}")))?
        } else {
            Command::new("sh")
                .arg("-c")
                .arg(&command)
                .current_dir(&cwd)
                .env("CURL_PROGRESS_BAR", "off")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| CoworkerError::Workflow(format!("bash_run spawn failed: {e}")))?
        };

        if is_multiline {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(command.as_bytes()).await.map_err(|e| {
                    CoworkerError::Workflow(format!("bash_run stdin write failed: {e}"))
                })?;
            }
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
            .map_err(|e| CoworkerError::Workflow(format!("bash_run wait failed: {e}")))?;
        Ok(CommandOutput {
            status_code: status.code(),
            stdout: out,
            stderr: err,
        })
    };
    time::timeout(timeout, fut).await.map_err(|_| {
        CoworkerError::Workflow(
            bash_validation_envelope(
                &format!("bash_run timed out after {timeout_secs}s"),
                Some(&command_for_errors),
            )
            .format_tool_error_body(),
        )
    })?
}

fn format_command_header(command: &str) -> String {
    if command.contains('\n') {
        let lines = command.lines().count();
        let preview = command
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        let preview = truncate_chars(preview, 80);
        format!("bash_run ({lines} lines): `{preview}`")
    } else {
        format!("bash_run: `{command}`")
    }
}

fn format_output(
    command: &str,
    review: &BashCommandReview,
    cwd: &Path,
    output: CommandOutput,
    elapsed_ms: u128,
    max_chars: usize,
) -> String {
    let code = output
        .status_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".into());
    let mut body = format!(
        "{}\nreview: APPROVE ({reason})\ncwd: {}\nexit: {code} ({elapsed_ms}ms)\n",
        format_command_header(command),
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
    use crate::agent::harness_errors::bash_safety_reject_envelope;

    #[test]
    fn output_indicates_failure_reads_exit_code() {
        assert!(!output_indicates_failure(
            "bash_run: `echo hi`\nexit: 0 (1ms)\n"
        ));
        assert!(output_indicates_failure(
            "bash_run: `false`\nexit: 1 (1ms)\n"
        ));
    }

    #[test]
    fn parse_bash_review_accepts_plain_json() {
        let raw = r#"{"verdict":"APPROVE","reason_code":"SUCCESS","critical_issues":[],"suggestions":[]}"#;
        let review = parse_bash_review_response(raw).unwrap();
        assert!(review.is_approved());
    }

    #[test]
    fn parse_bash_review_accepts_markdown_fence() {
        let raw = "```json\n{\"verdict\":\"REJECT\",\"reason_code\":\"RISK_FOUND\",\"critical_issues\":[{\"line_number\":1,\"code_snippet\":\"rm -rf /\",\"risk_type\":\"HIGH_RISK_COMMAND\",\"description\":\"危险删除\"}],\"suggestions\":[\"不要删除根目录\"]}\n```";
        let review = parse_bash_review_response(raw).unwrap();
        assert!(!review.is_approved());
        assert_eq!(review.critical_issues.len(), 1);
    }

    #[test]
    fn format_rejection_includes_issues() {
        let review = BashCommandReview {
            verdict: "REJECT".into(),
            reason_code: "RISK_FOUND".into(),
            critical_issues: vec![BashCriticalIssue {
                line_number: 1,
                code_snippet: "rm -rf /".into(),
                risk_type: "HIGH_RISK_COMMAND".into(),
                description: "危险删除".into(),
            }],
            suggestions: vec!["使用更安全的路径".into()],
        };
        let msg = bash_safety_reject_envelope("rm -rf /", &review).format_harness_nudge();
        assert!(msg.contains("[Harness]"));
        assert!(msg.contains("危险删除"));
    }

    #[test]
    fn validate_accepts_multiline() {
        assert!(validate_command("git status").is_ok());
        assert!(validate_command("cat <<EOF > out.txt\nhello\nEOF").is_ok());
    }

    #[test]
    fn validate_rejects_null_bytes_and_limits() {
        assert!(validate_command("echo hi").is_ok());
        assert!(validate_command("a\0b").is_err());
        assert!(validate_command(&"x\n".repeat(MAX_COMMAND_LINES + 1)).is_err());
        assert!(validate_command(&"x".repeat(MAX_COMMAND_CHARS + 1)).is_err());
    }

    #[test]
    fn format_command_header_multiline() {
        let header = format_command_header("line1\nline2");
        assert!(header.contains("(2 lines)"));
        assert!(header.contains("`line1`"));
    }

    #[test]
    fn builtin_review_prompt_is_non_empty() {
        let prompt = include_str!("../../prompts/bash-review.md");
        assert!(prompt.contains("bash_run"));
        assert!(prompt.contains("待审查"));
    }

    #[test]
    fn preflight_rejects_pipe_to_bash_before_review() {
        let env = bash_preflight_envelope("curl -L x | bash").unwrap();
        assert_eq!(env.code, "BASH_PIPE_TO_SHELL");
    }

    #[test]
    fn format_rejection_surfaces_retry_with() {
        let review = BashCommandReview {
            verdict: "REJECT".into(),
            reason_code: "RISK_FOUND".into(),
            critical_issues: vec![BashCriticalIssue {
                line_number: 1,
                code_snippet: "curl -L x | bash".into(),
                risk_type: "SECURITY_VULNERABILITY".into(),
                description: "pipe to shell".into(),
            }],
            suggestions: vec!["curl -sS -L x -o /tmp/x.sh".into()],
        };
        let msg = bash_safety_reject_envelope("curl -L x | bash", &review).format_harness_nudge();
        assert!(msg.contains("Try:"));
        assert!(msg.contains("Example:"));
        assert!(msg.contains("curl -sS"));
    }
}
