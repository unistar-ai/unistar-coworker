//! Headless Chromium fetch for `web_fetch` when `browser: true`.

use std::path::Path;
use std::time::Duration;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::detection::{default_executable, DetectionOptions};
use futures_util::StreamExt;

use crate::agent::harness_errors::{self, parse_error_line, workflow_error, ErrorEnvelope};
use crate::error::CoworkerError;
use crate::agent::web_fetch_tool::{FetchedContent, WEB_FETCH_TOOL};
use crate::config::WebFetchToolConfig;
use crate::error::Result;

pub async fn fetch_page_with_chromium(
    config: &WebFetchToolConfig,
    url: &str,
    source_label: &str,
) -> Result<FetchedContent> {
    let timeout = Duration::from_secs(config.browser_timeout_secs.max(5));
    fetch_page_inner(config, url, source_label, timeout).await
}

async fn fetch_page_inner(
    config: &WebFetchToolConfig,
    url: &str,
    source_label: &str,
    timeout: Duration,
) -> Result<FetchedContent> {
    let user_agent = effective_user_agent(config);
    let mut builder = BrowserConfig::builder()
        .new_headless_mode()
        .arg("--disable-blink-features=AutomationControlled")
        .arg("--disable-dev-shm-usage")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(format!("--user-agent={user_agent}"));
    if let Some(path) = resolve_chromium_path(config) {
        builder = builder.chrome_executable(path);
    }

    let (mut browser, mut handler) = Browser::launch(builder.build().map_err(browser_launch_err)?)
        .await
        .map_err(browser_launch_err)?;

    // Pump CDP events until the browser shuts down. chromiumoxide ignores unknown CDP
    // messages (Chrome often sends events ahead of chromiumoxide_cdp schemas).
    let mut pump = tokio::spawn(async move {
        while let Some(ev) = handler.next().await {
            if let Err(e) = ev {
                tracing::debug!("chromium CDP handler stopped: {e}");
                break;
            }
        }
    });

    // Timeout only the fetch work — browser shutdown must always run (avoids Drop warning).
    let fetch = async {
        let page = browser
            .new_page(url)
            .await
            .map_err(|e| browser_nav_err(url, &e.to_string()))?;
        let _ = page.wait_for_navigation().await;

        if config.browser_wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(config.browser_wait_ms)).await;
        }

        wait_for_readable_body(&page, config).await?;

        let html = page
            .content()
            .await
            .map_err(|e| browser_nav_err(url, &e.to_string()))?;
        let body_text = page
            .evaluate("document.body ? document.body.innerText : ''")
            .await
            .map_err(|e| browser_nav_err(url, &e.to_string()))?
            .into_value::<String>()
            .unwrap_or_default();

        Ok(FetchedContent {
            source_label: source_label.to_string(),
            body: html,
            content_type: "text/html".into(),
            status_line: "status: 200 (chromium)\nrender: headless\n".into(),
            base_url: Some(url.to_string()),
            body_text_override: Some(body_text),
        })
    };

    let result = match tokio::time::timeout(timeout, fetch).await {
        Ok(inner) => inner,
        Err(_) => Err(workflow_error(browser_timeout_envelope(
            url,
            config.browser_timeout_secs,
        ))),
    };

    shutdown_browser(&mut browser, &mut pump).await;
    result
}

/// Gracefully stop headless Chrome and reap the child process before `Browser` is dropped.
async fn shutdown_browser(browser: &mut Browser, pump: &mut tokio::task::JoinHandle<()>) {
    if browser.close().await.is_err() {
        tracing::debug!("chromium close failed; killing child process");
        let _ = browser.kill().await;
    }
    let _ = pump.await;
    if let Err(e) = browser.wait().await {
        tracing::debug!("chromium wait after close: {e}");
    }
}

fn resolve_chromium_path(config: &WebFetchToolConfig) -> Option<std::path::PathBuf> {
    if let Some(path) = config
        .chromium_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(path.into());
    }
    default_executable(DetectionOptions::default()).ok()
}

async fn wait_for_readable_body(
    page: &chromiumoxide::Page,
    config: &WebFetchToolConfig,
) -> Result<()> {
    let max_polls = 10u32;
    let poll_ms = config.browser_wait_ms.clamp(500, 2_000);
    for _ in 0..max_polls {
        let html = page
            .content()
            .await
            .map_err(|e| browser_nav_err("page", &e.to_string()))?;
        let text = page
            .evaluate("document.body ? document.body.innerText.trim().length : 0")
            .await
            .map_err(|e| browser_nav_err("page", &e.to_string()))?
            .into_value::<f64>()
            .unwrap_or(0.0) as usize;
        let still_challenge = html.contains("zh-zse-ck") || html.contains("zse-ck");
        if text >= config.spa_empty_chars && !still_challenge {
            return Ok(());
        }
        if text >= config.spa_empty_chars && still_challenge {
            // Challenge script ran but left marker — body may still be usable.
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }
    Ok(())
}

pub fn file_url_for_path(path: &Path) -> Result<String> {
    let canonical = path.canonicalize().map_err(|e| {
        harness_errors::web_fetch_validation_error(
            "WEB_LOCAL_PATH",
            format!("cannot resolve path for browser: {e}"),
            "Use a workspace-relative HTML path without `..`",
        )
    })?;
    Ok(format!("file://{}", canonical.display()))
}

fn effective_user_agent(config: &WebFetchToolConfig) -> String {
    let ua = config.user_agent.trim();
    // Plain HTTP uses the agent UA; headless Chrome should look like a real browser.
    if ua.is_empty() || ua.starts_with("unistar-coworker/") {
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".into()
    } else {
        ua.to_string()
    }
}

fn browser_launch_err(e: impl std::fmt::Display) -> crate::error::CoworkerError {
    workflow_error(ErrorEnvelope {
        code: "WEB_FETCH_LAUNCH_FAILED".into(),
        tool_name: WEB_FETCH_TOOL.into(),
        what: "Failed to launch headless Chromium".into(),
        why: e.to_string(),
        try_steps: vec![
            "Install Google Chrome or Chromium".into(),
            "Set chat.web_fetch.chromium_path to the chrome/chromium binary".into(),
            "On macOS: /Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into(),
        ],
        example: Some(harness_errors::web_fetch_tool_example(
            "https://www.zhihu.com/question/1",
            "full",
            true,
        )),
        detail: None,
    })
}

fn browser_nav_err(url: &str, err: &str) -> crate::error::CoworkerError {
    workflow_error(ErrorEnvelope {
        code: "WEB_FETCH_NAV_FAILED".into(),
        tool_name: WEB_FETCH_TOOL.into(),
        what: "Headless browser failed to load the page".into(),
        why: err.to_string(),
        try_steps: vec![
            "Increase chat.web_fetch.browser_timeout_secs or browser_wait_ms".into(),
            "Retry with mode=metadata first".into(),
        ],
        example: Some(harness_errors::web_fetch_tool_example(url, "full", true)),
        detail: Some(format!("url: {url}")),
    })
}

/// Error codes from headless Chromium launch, navigation, or timeout — eligible for HTTP fallback.
pub const BROWSER_FETCH_FAILURE_CODES: &[&str] = &[
    "WEB_FETCH_LAUNCH_FAILED",
    "WEB_FETCH_NAV_FAILED",
    "WEB_FETCH_TIMEOUT",
];

pub fn is_browser_fetch_failure(err: &CoworkerError) -> bool {
    let CoworkerError::Workflow(body) = err else {
        return false;
    };
    parse_error_line(body)
        .is_some_and(|p| BROWSER_FETCH_FAILURE_CODES.contains(&p.code.as_str()))
}

pub fn browser_failure_brief(err: &CoworkerError) -> String {
    let CoworkerError::Workflow(body) = err else {
        return "headless browser failed".into();
    };
    let Some(parsed) = parse_error_line(body) else {
        return "headless browser failed".into();
    };
    match parsed.code.as_str() {
        "WEB_FETCH_LAUNCH_FAILED" => "headless Chromium launch failed".into(),
        "WEB_FETCH_NAV_FAILED" => "headless browser navigation failed".into(),
        "WEB_FETCH_TIMEOUT" => "headless browser timed out".into(),
        _ => parsed.message,
    }
}

fn browser_timeout_envelope(url: &str, secs: u64) -> ErrorEnvelope {
    ErrorEnvelope {
        code: "WEB_FETCH_TIMEOUT".into(),
        tool_name: WEB_FETCH_TOOL.into(),
        what: "Headless browser timed out".into(),
        why: format!("Exceeded {secs}s while loading or rendering"),
        try_steps: vec![
            "Increase chat.web_fetch.browser_timeout_secs".into(),
            "Increase chat.web_fetch.browser_wait_ms for slow JS challenges".into(),
        ],
        example: Some(harness_errors::web_fetch_tool_example(
            url, "metadata", true,
        )),
        detail: Some(format!("url: {url}")),
    }
}
