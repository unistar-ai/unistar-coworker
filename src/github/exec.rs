use std::process::Stdio;
use std::time::Duration;

use regex::Regex;
use tokio::process::Command;
use tokio::time::sleep;

use super::error::{format_tool_error, ErrCode};
use crate::error::{CoworkerError, Result};

const ERR_DETAIL_BUDGET: usize = 2_000;
const DEFAULT_ATTEMPTS: u32 = 3;
const DEFAULT_GIT: &str = "git";

pub struct RunResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub err: Option<String>,
}

impl RunResult {
    pub fn combined(&self) -> String {
        match (self.stderr.is_empty(), self.stdout.is_empty()) {
            (true, _) => self.stdout.clone(),
            (false, true) => self.stderr.clone(),
            (false, false) => format!("{}\n{}", self.stdout, self.stderr),
        }
    }

    pub fn transient(&self) -> bool {
        static TRANSIENT: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        static RATE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let transient = TRANSIENT.get_or_init(|| {
            Regex::new(r"(?i)HTTP 50[234]|bad gateway|service unavailable|gateway timeout|server error|connection reset|unexpected EOF|TLS handshake timeout").unwrap()
        });
        let rate = RATE.get_or_init(|| {
            Regex::new(r"(?i)rate limit|HTTP 429|too many requests").unwrap()
        });
        self.err.is_some() && transient.is_match(&self.combined()) && !rate.is_match(&self.combined())
    }

    pub fn wrap(&self, action: &str) -> CoworkerError {
        let detail = tail(&self.combined(), ERR_DETAIL_BUDGET);
        let low = self.combined().to_ascii_lowercase();

        if self.err.as_deref() == Some("not_found") {
            return CoworkerError::Other(anyhow::anyhow!(format_tool_error(
                ErrCode::Unavailable,
                &format!("{action}: gh is not installed or not on PATH"),
                "Install GitHub CLI (https://cli.github.com/) and ensure `gh` is on PATH",
            )));
        }

        if low.contains("rate limit") || low.contains("http 429") {
            return CoworkerError::Other(anyhow::anyhow!(format_tool_error(
                ErrCode::RateLimit,
                &format!("{action}: GitHub rate limit reached"),
                "Wait at least a minute, then retry",
            )));
        }

        if low.contains("gh auth login")
            || low.contains("authentication")
            || low.contains("not logged in")
            || low.contains("http 401")
            || low.contains("bad credentials")
        {
            return CoworkerError::Other(anyhow::anyhow!(format_tool_error(
                ErrCode::Auth,
                &format!("{action}: GitHub authentication failed"),
                "Run `gh auth login` or set GH_TOKEN / GITHUB_TOKEN",
            )));
        }

        if low.contains("could not resolve to a repository")
            || low.contains("http 404")
            || low.contains("not found")
        {
            return CoworkerError::Other(anyhow::anyhow!(format_tool_error(
                ErrCode::NotFound,
                &format!("{action}: repository, PR, or run not found"),
                "Check owner/repo and IDs",
            )));
        }

        if low.contains("http 403")
            || low.contains("permission")
            || low.contains("forbidden")
        {
            return CoworkerError::Other(anyhow::anyhow!(format_tool_error(
                ErrCode::Forbidden,
                &format!("{action}: permission denied"),
                "The token lacks access to this repository",
            )));
        }

        if self.transient() {
            return CoworkerError::Other(anyhow::anyhow!(format_tool_error(
                ErrCode::Transient,
                &format!("{action}: transient GitHub error"),
                "Retry the same call",
            )));
        }

        let exit = self
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".into());
        CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::Generic,
            &format!("{action} (exit {exit})"),
            &detail,
        )))
    }
}

fn tail(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let skip = s.chars().count().saturating_sub(limit);
    s.chars().skip(skip).collect()
}

pub struct GhExec {
    pub gh: String,
    pub timeout: Duration,
}

/// Git subprocess runner (separate from GhExec so harness construction stays unchanged).
pub struct GitExec {
    pub git: String,
    pub timeout: Duration,
}

impl GitExec {
    pub fn from_gh(exec: &GhExec) -> Self {
        Self {
            git: DEFAULT_GIT.to_string(),
            timeout: exec.timeout,
        }
    }

    pub async fn run(&self, dir: Option<&str>, args: &[&str]) -> RunResult {
        let mut cmd = Command::new(&self.git);
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = dir {
            cmd.current_dir(dir);
        }
        match tokio::time::timeout(self.timeout, cmd.output()).await {
            Ok(Ok(output)) => RunResult {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                exit_code: output.status.code(),
                err: if output.status.success() {
                    None
                } else {
                    Some("exit_error".into())
                },
            },
            Ok(Err(e)) => RunResult {
                stdout: String::new(),
                stderr: e.to_string(),
                exit_code: None,
                err: if e.kind() == std::io::ErrorKind::NotFound {
                    Some("not_found".into())
                } else {
                    Some(e.to_string())
                },
            },
            Err(_) => RunResult {
                stdout: String::new(),
                stderr: format!("timed out after {}s", self.timeout.as_secs()),
                exit_code: None,
                err: Some("timeout".into()),
            },
        }
    }

    pub async fn run_gh_in_dir(&self, gh: &GhExec, dir: &str, args: &[&str]) -> RunResult {
        let mut cmd = Command::new(&gh.gh);
        cmd.args(args)
            .current_dir(dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        match tokio::time::timeout(gh.timeout, cmd.output()).await {
            Ok(Ok(output)) => RunResult {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                exit_code: output.status.code(),
                err: if output.status.success() {
                    None
                } else {
                    Some("exit_error".into())
                },
            },
            Ok(Err(e)) => RunResult {
                stdout: String::new(),
                stderr: e.to_string(),
                exit_code: None,
                err: if e.kind() == std::io::ErrorKind::NotFound {
                    Some("not_found".into())
                } else {
                    Some(e.to_string())
                },
            },
            Err(_) => RunResult {
                stdout: String::new(),
                stderr: format!("timed out after {}s", gh.timeout.as_secs()),
                exit_code: None,
                err: Some("timeout".into()),
            },
        }
    }
}

impl GhExec {
    pub async fn run_retry(&self, args: &[&str]) -> RunResult {
        let mut last = self.run(args).await;
        for attempt in 1..DEFAULT_ATTEMPTS {
            if !last.transient() {
                return last;
            }
            tracing::debug!("transient gh failure, retry {attempt}");
            sleep(Duration::from_secs(attempt as u64 * 2)).await;
            last = self.run(args).await;
        }
        last
    }

    pub async fn run(&self, args: &[&str]) -> RunResult {
        let mut cmd = Command::new(&self.gh);
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        match tokio::time::timeout(self.timeout, cmd.output()).await {
            Ok(Ok(output)) => RunResult {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                exit_code: output.status.code(),
                err: if output.status.success() {
                    None
                } else {
                    Some("exit_error".into())
                },
            },
            Ok(Err(e)) => RunResult {
                stdout: String::new(),
                stderr: e.to_string(),
                exit_code: None,
                err: if e.kind() == std::io::ErrorKind::NotFound {
                    Some("not_found".into())
                } else {
                    Some(e.to_string())
                },
            },
            Err(_) => RunResult {
                stdout: String::new(),
                stderr: format!("timed out after {}s", self.timeout.as_secs()),
                exit_code: None,
                err: Some("timeout".into()),
            },
        }
    }

    pub fn into_result(res: RunResult, action: &str) -> Result<String> {
        if res.err.is_some() {
            return Err(res.wrap(action));
        }
        Ok(res.stdout)
    }
}
