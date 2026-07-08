use std::env;
use std::time::Duration;

use reqwest::Client;
use serde_json::{json, Value};

use super::args::{optional_str, require_str};
use super::error::{format_tool_error, format_tool_ok, ErrCode};
use crate::error::{CoworkerError, Result};

const SLACK_WEBHOOK_TIMEOUT: Duration = Duration::from_secs(15);

pub async fn notify_post_slack(_exec: &super::exec::GhExec, args: &Value) -> Result<String> {
    let text = require_str(args, "text")?;
    if text.is_empty() {
        return Err(CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::Validation,
            "text is empty",
            "pass a short summary, not raw logs",
        ))));
    }

    let mut url = optional_str(args, "webhook_url").unwrap_or_default();
    if url.is_empty() {
        url = env::var("SLACK_WEBHOOK_URL")
            .unwrap_or_default()
            .trim()
            .to_string();
    }
    if url.is_empty() {
        return Err(CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::Validation,
            "no Slack webhook URL configured",
            "set SLACK_WEBHOOK_URL on the MCP server or pass webhook_url",
        ))));
    }

    let client = Client::builder()
        .timeout(SLACK_WEBHOOK_TIMEOUT)
        .build()
        .map_err(|e| CoworkerError::Other(anyhow::anyhow!(e.to_string())))?;

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&json!({ "text": text }))
        .send()
        .await
        .map_err(|e| {
            let hint = if e.is_timeout() {
                "Slack webhook timed out — retry once"
            } else {
                "verify webhook URL and network reachability"
            };
            let code = if e.is_timeout() {
                ErrCode::Transient
            } else {
                ErrCode::Generic
            };
            CoworkerError::Other(anyhow::anyhow!(format_tool_error(
                code,
                &e.to_string(),
                hint,
            )))
        })?;

    let status = resp.status();
    if status.is_success() {
        let preview = if text.chars().count() > 80 {
            format!("{}...", text.chars().take(77).collect::<String>())
        } else {
            text.clone()
        };
        return Ok(format_tool_ok(&format!(
            "Slack message posted ({} chars): {preview:?}",
            text.len()
        )));
    }

    let (code, hint) = if status.as_u16() == 429 {
        (ErrCode::RateLimit, "Slack rate-limited — wait and retry")
    } else if status.is_server_error() {
        (ErrCode::Transient, "Slack server error — retry once")
    } else {
        (
            ErrCode::Generic,
            "verify webhook URL is valid and channel exists",
        )
    };
    Err(CoworkerError::Other(anyhow::anyhow!(format_tool_error(
        code,
        &format!("Slack webhook returned HTTP {}", status.as_u16()),
        hint,
    ))))
}
