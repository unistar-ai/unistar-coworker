use serde_json::Value;

use super::ci_fingerprint::{self, RunFailureAnalysis};
use super::exec::GhExec;
use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureVerdict {
    Test,
    Infra,
    Auth,
    Timeout,
    External,
    Unknown,
}

impl FailureVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Infra => "infra",
            Self::Auth => "auth",
            Self::Timeout => "timeout",
            Self::External => "external_ci",
            Self::Unknown => "unknown",
        }
    }
}

pub fn classify_failure(analysis: &RunFailureAnalysis) -> (FailureVerdict, &'static str) {
    let corpus = [
        analysis.error_sig.as_str(),
        analysis.test_name.as_str(),
        analysis.job.as_str(),
        analysis.step.as_str(),
        analysis.workflow.as_str(),
    ]
    .join(" ")
    .to_ascii_lowercase();

    const RULES: &[(&str, FailureVerdict, &[&str])] = &[
        (
            "external_ci_hint",
            FailureVerdict::External,
            &[
                "external ci",
                "status context",
                "jenkins",
                "codecov",
                "third-party check",
            ],
        ),
        (
            "timeout",
            FailureVerdict::Timeout,
            &[
                "timeout",
                "timed out",
                "deadline exceeded",
                "context deadline",
                "i/o timeout",
            ],
        ),
        (
            "auth",
            FailureVerdict::Auth,
            &[
                "401",
                "403",
                "unauthorized",
                "authentication failed",
                "permission denied",
                "bad credentials",
                "invalid token",
                "access denied",
            ],
        ),
        (
            "infra",
            FailureVerdict::Infra,
            &[
                "connection refused",
                "connection reset",
                "no space left",
                "out of memory",
                "oom",
                "docker",
                "registry unreachable",
                "503 service unavailable",
                "502 bad gateway",
                "504 gateway",
                "network is unreachable",
                "cannot connect",
                "runner lost communication",
                "pod evicted",
            ],
        ),
    ];

    for (id, verdict, subs) in RULES {
        for sub in *subs {
            if corpus.contains(sub) {
                return (*verdict, id);
            }
        }
    }

    if !analysis.test_name.trim().is_empty() {
        return (FailureVerdict::Test, "named_test_failure");
    }

    let low = analysis.error_sig.to_ascii_lowercase();
    if low.contains("assert")
        || low.contains("expect")
        || low.contains("panic:")
        || low.contains("failed:")
    {
        return (FailureVerdict::Test, "test_assertion");
    }

    (FailureVerdict::Unknown, "no_rule_match")
}

pub fn format_policy_classification(analysis: &RunFailureAnalysis) -> String {
    let (verdict, rule_id) = classify_failure(analysis);
    let mut out = format!("VERDICT: {}\n", verdict.as_str());
    out.push_str(&format!("Matched rule: {rule_id}\n"));
    out.push_str(&format!("Run {}  {}\n", analysis.run_id, analysis.workflow));
    if !analysis.job.is_empty() {
        out.push_str(&format!("Job: {}\n", analysis.job));
    }
    if !analysis.test_name.is_empty() {
        out.push_str(&format!("Test: {}\n", analysis.test_name));
    }
    if !analysis.error_sig.is_empty() {
        out.push_str(&format!("Error signature: {}\n", analysis.error_sig));
    }
    out.push_str(&format!("Fingerprint: {}\n", analysis.fingerprint));

    match verdict {
        FailureVerdict::Timeout | FailureVerdict::Infra => {
            out.push_str("Next: ci_rerun_workflow if this looks transient; ci_compare_runs after rerun.");
        }
        FailureVerdict::Auth => {
            out.push_str("Next: fix credentials/secrets — do not rerun until auth is resolved.");
        }
        FailureVerdict::External => {
            out.push_str(&format!(
                "Error class: {}\n",
                super::error::ErrCode::ExternalCi.as_str()
            ));
            out.push_str("Next: ci_list_external_checks — do not call ci_get_failed_logs for external CI.");
        }
        FailureVerdict::Test => {
            out.push_str("Next: ci_get_failed_logs for details; avoid blind rerun.");
        }
        FailureVerdict::Unknown => {
            out.push_str("Next: ci_get_failed_logs then re-run policy_classify_failure.");
        }
    }
    out.trim().to_string()
}

use super::args::{require_str, require_u64};
use super::error::{format_tool_error, ErrCode};

pub async fn policy_classify_failure(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo").map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::Validation,
            &e.to_string(),
            "pass repo and run_id",
        )))
    })?;
    let run_id = require_u64(args, "run_id").map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::Validation,
            &e.to_string(),
            "pass run_id from ci_analyze_pr_failures or ci_failure_fingerprint",
        )))
    })?;

    let analysis = ci_fingerprint::analyze_run_failure(exec, &repo, run_id)
        .await
        .map_err(|e| {
            CoworkerError::Other(anyhow::anyhow!(format_tool_error(
                ErrCode::NotFound,
                &e.to_string(),
                "confirm run_id with ci_get_run_summary",
            )))
        })?;

    Ok(format_policy_classification(&analysis))
}
