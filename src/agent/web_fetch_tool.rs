//! Read-only web fetch (`web_fetch`) — fetch page text for the agent.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT, CONTENT_TYPE};
use reqwest::Url;
use serde_json::Value;

use crate::agent::context::truncate_chars;
use crate::agent::file_tools;
use crate::agent::harness_errors::{self, workflow_error, ErrorEnvelope};
use crate::config::WebFetchToolConfig;
use crate::error::{CoworkerError, Result};

pub const WEB_FETCH_TOOL: &str = "web_fetch";

const DEFAULT_MAX_CHARS: usize = 32_000;
const METADATA_DEFAULT_MAX_CHARS: usize = 8_000;
const DEFAULT_USER_AGENT: &str = "unistar-coworker/1.0 (+local coding agent)";

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static RESPONSE_CACHE: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();
static HREF_RE: OnceLock<Regex> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebFetchMode {
    Full,
    Metadata,
    Links,
}

struct CacheEntry {
    at: Instant,
    payload: CachedFetch,
}

#[derive(Clone)]
struct CachedFetch {
    source_label: String,
    body: String,
    content_type: String,
    status_line: String,
}

struct PageMeta {
    title: Option<String>,
    description: Option<String>,
    headings: Vec<String>,
    links: Vec<String>,
    body_text: String,
}

pub fn is_web_fetch_tool(name: &str) -> bool {
    name == WEB_FETCH_TOOL || name == "web_browser"
}

pub async fn execute_web_fetch_tool(
    config: &WebFetchToolConfig,
    workspace: &Path,
    args: &Value,
) -> Result<String> {
    let raw_url = args
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            harness_errors::web_fetch_validation_error(
                "WEB_MISSING_URL",
                "web_fetch needs non-empty url",
                "Pass url as http(s)://… or a workspace-relative HTML path",
            )
        })?;

    let mode = parse_mode(args)?;
    let max_chars = effective_max_chars(config, mode, args);
    let use_browser = parse_browser_arg(args);

    let normalized = normalize_url_input(raw_url);
    let fetched = if use_browser {
        fetch_with_browser(config, workspace, &normalized).await?
    } else if is_remote_url(&normalized) {
        fetch_remote_page(config, &normalized).await?
    } else {
        load_local_html(workspace, &normalized)?
    };

    let mut page = process_content(
        &fetched.body,
        &fetched.content_type,
        fetched.base_url.as_deref(),
        config.max_links,
    )?;
    if let Some(body) = fetched.body_text_override {
        page.body_text = body;
    }

    let mut out = format!("web_fetch: {}\n", fetched.source_label);
    if fetched.status_line.contains("chromium") {
        out.push_str("engine: headless-chromium\n");
    }
    if !fetched.status_line.is_empty() {
        out.push_str(&fetched.status_line);
    }
    out.push_str(&format!("content-type: {}\n", fetched.content_type));
    if let Some(title) = &page.title {
        out.push_str(&format!("title: {title}\n"));
    }
    if let Some(desc) = &page.description {
        out.push_str(&format!("description: {desc}\n"));
    }
    if !page.headings.is_empty() {
        out.push_str("headings:\n");
        for h in &page.headings {
            out.push_str(&format!("  - {h}\n"));
        }
    }
    if !page.links.is_empty() {
        out.push_str("links:\n");
        for link in &page.links {
            out.push_str(&format!("  - {link}\n"));
        }
    }

    match mode {
        WebFetchMode::Links => {}
        WebFetchMode::Metadata => {}
        WebFetchMode::Full => {
            if page.body_text.chars().count() < config.spa_empty_chars {
                out.push_str(&format!(
                    "warning: body very short ({} chars) — page may require JavaScript; \
                     try read_file on source or bash_run curl for API JSON\n",
                    page.body_text.chars().count()
                ));
            }
            out.push_str("\n---\n");
            out.push_str(&truncate_chars(&page.body_text, max_chars));
        }
    }

    Ok(out.trim_end().to_string())
}

fn parse_mode(args: &Value) -> Result<WebFetchMode> {
    match args.get("mode").and_then(|v| v.as_str()).unwrap_or("full") {
        "full" => Ok(WebFetchMode::Full),
        "metadata" => Ok(WebFetchMode::Metadata),
        "links" => Ok(WebFetchMode::Links),
        other => Err(harness_errors::web_fetch_validation_error(
            "WEB_INVALID_MODE",
            format!("web_fetch unknown mode `{other}`"),
            "Use mode: full | metadata | links",
        )),
    }
}

fn parse_browser_arg(args: &Value) -> bool {
    args.get("browser")
        .map(|v| {
            v.as_bool().unwrap_or(false)
                || v.as_str().is_some_and(|s| {
                    matches!(
                        s.trim().to_ascii_lowercase().as_str(),
                        "true" | "1" | "yes"
                    )
                })
        })
        .unwrap_or(false)
}

async fn fetch_with_browser(
    config: &WebFetchToolConfig,
    workspace: &Path,
    url_or_path: &str,
) -> Result<FetchedContent> {
    use crate::agent::web_fetch_chromium::{fetch_page_with_chromium, file_url_for_path};

    if is_remote_url(url_or_path) {
        validate_remote_url(url_or_path, config)?;
        return fetch_page_with_chromium(config, url_or_path, url_or_path).await;
    }
    let resolved = file_tools::resolve_workspace_path(workspace, url_or_path).map_err(|e| {
        harness_errors::web_fetch_validation_error(
            "WEB_LOCAL_PATH",
            e.to_string(),
            "Use a workspace-relative HTML path without `..`",
        )
    })?;
    if !resolved.is_file() {
        return Err(harness_errors::web_fetch_validation_error(
            "WEB_NOT_FILE",
            format!("web_fetch: {url_or_path:?} is not a file"),
            "Point url at an HTML file under chat.workspace",
        ));
    }
    let file_url = file_url_for_path(&resolved)?;
    let label = file_tools::display_relative(workspace, &resolved);
    fetch_page_with_chromium(config, &file_url, &label).await
}

fn effective_max_chars(config: &WebFetchToolConfig, mode: WebFetchMode, args: &Value) -> usize {
    let default = match mode {
        WebFetchMode::Full => config.max_content_chars.min(DEFAULT_MAX_CHARS),
        WebFetchMode::Metadata | WebFetchMode::Links => METADATA_DEFAULT_MAX_CHARS,
    };
    args.get("max_chars")
        .and_then(|v| v.as_u64())
        .map(|n| n.clamp(500, config.max_content_chars as u64) as usize)
        .unwrap_or(default)
}

fn normalize_url_input(input: &str) -> String {
    let s = input.trim();
    if is_remote_url(s) {
        return s.to_string();
    }
    if s.starts_with("localhost:")
        || s.starts_with("127.0.0.1:")
        || s.starts_with("[::1]:")
    {
        return format!("http://{s}");
    }
    s.to_string()
}

fn is_remote_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

fn validate_remote_url(url: &str, config: &WebFetchToolConfig) -> Result<Url> {
    let parsed = Url::parse(url).map_err(|e| {
        harness_errors::web_fetch_validation_error(
            "WEB_INVALID_URL",
            format!("web_fetch invalid url: {e}"),
            "Use http(s)://… or localhost:PORT (with allow_localhost) or a workspace HTML path",
        )
    })?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(harness_errors::web_fetch_validation_error(
                "WEB_UNSUPPORTED_SCHEME",
                format!("web_fetch only supports http/https URLs (got {other}:)"),
                "Use http(s):// for remote URLs; local HTML via workspace-relative path",
            ));
        }
    }
    if let Some(host) = parsed.host_str() {
        if is_blocked_host(host, config.allow_localhost) {
            return Err(workflow_error(web_fetch_ssrf_envelope(host, config.allow_localhost)));
        }
    }
    Ok(parsed)
}

fn is_blocked_host(host: &str, allow_localhost: bool) -> bool {
    if allow_localhost && (host == "localhost" || host.ends_with(".localhost")) {
        return false;
    }
    if host == "localhost" {
        return !allow_localhost;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_private_or_loopback(ip) && !(allow_localhost && ip.is_loopback());
    }
    false
}

fn is_private_or_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.segments()[0] & 0xfe00 == 0xfc00
                || v6.segments()[0] & 0xffc0 == 0xfe80
        }
    }
}

fn http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .expect("web_fetch HTTP client")
    })
}

fn response_cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    RESPONSE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) struct FetchedContent {
    pub source_label: String,
    pub body: String,
    pub content_type: String,
    pub status_line: String,
    pub base_url: Option<String>,
    pub body_text_override: Option<String>,
}

async fn fetch_remote_page(config: &WebFetchToolConfig, url: &str) -> Result<FetchedContent> {
    validate_remote_url(url, config)?;

    if config.cache_ttl_secs > 0 {
        if let Some(cached) = cache_get(url, config.cache_ttl_secs) {
            return Ok(cached);
        }
    }

    let user_agent = if config.user_agent.trim().is_empty() {
        DEFAULT_USER_AGENT
    } else {
        config.user_agent.trim()
    };

    let response = http_client()
        .get(url)
        .timeout(Duration::from_secs(config.timeout_secs))
        .header(
            USER_AGENT,
            HeaderValue::from_str(user_agent)
                .map_err(|e| CoworkerError::Workflow(format!("web_fetch bad user_agent: {e}")))?,
        )
        .send()
        .await
        .map_err(|e| workflow_error(web_fetch_envelope(url, &e.to_string(), None)))?;

    let status = response.status();
    let headers: HeaderMap = response.headers().clone();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| workflow_error(web_fetch_envelope(url, &e.to_string(), Some(status.as_u16()))))?;

    if bytes.len() > config.max_download_bytes {
        return Err(workflow_error(web_fetch_too_large_envelope(
            bytes.len(),
            config.max_download_bytes,
        )));
    }

    if status.as_u16() == 403 || status.as_u16() == 401 {
        let snippet = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
        let challenge = snippet.contains("zh-zse-ck") || snippet.contains("zse-ck");
        return Err(workflow_error(web_fetch_forbidden_envelope(
            status.as_u16(),
            challenge,
        )));
    }

    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    if looks_binary(&bytes) && !content_type.contains("json") && !content_type.contains("text") {
        return Err(workflow_error(web_fetch_binary_envelope(&content_type)));
    }

    let header_charset = parse_charset_from_content_type(&content_type);
    let sniff = if content_type.contains("html") {
        Some(String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).into_owned())
    } else {
        None
    };
    let body = decode_bytes(&bytes, header_charset.as_deref(), sniff.as_deref());

    let fetched = FetchedContent {
        source_label: url.to_string(),
        status_line: format!("status: {status}\n"),
        content_type: content_type.clone(),
        body,
        base_url: Some(url.to_string()),
        body_text_override: None,
    };

    if config.cache_ttl_secs > 0 && status.is_success() {
        cache_put(
            url,
            CachedFetch {
                source_label: fetched.source_label.clone(),
                body: fetched.body.clone(),
                content_type: fetched.content_type.clone(),
                status_line: fetched.status_line.clone(),
            },
        );
    }

    Ok(fetched)
}

fn cache_get(url: &str, ttl_secs: u64) -> Option<FetchedContent> {
    let cache = response_cache().lock().ok()?;
    let entry = cache.get(url)?;
    if entry.at.elapsed() > Duration::from_secs(ttl_secs) {
        return None;
    }
    let p = &entry.payload;
    Some(FetchedContent {
        source_label: p.source_label.clone(),
        body: p.body.clone(),
        content_type: p.content_type.clone(),
        status_line: format!("{}\ncache: hit\n", p.status_line.trim_end()),
        base_url: Some(url.to_string()),
        body_text_override: None,
    })
}

fn cache_put(url: &str, payload: CachedFetch) {
    if let Ok(mut cache) = response_cache().lock() {
        cache.insert(
            url.to_string(),
            CacheEntry {
                at: Instant::now(),
                payload,
            },
        );
    }
}

fn load_local_html(workspace: &Path, user_path: &str) -> Result<FetchedContent> {
    let resolved = file_tools::resolve_workspace_path(workspace, user_path).map_err(|e| {
        harness_errors::web_fetch_validation_error(
            "WEB_LOCAL_PATH",
            e.to_string(),
            "Use a workspace-relative HTML path without `..`",
        )
    })?;
    if !resolved.is_file() {
        return Err(harness_errors::web_fetch_validation_error(
            "WEB_NOT_FILE",
            format!("web_fetch: {user_path:?} is not a file"),
            "Point url at an HTML file under chat.workspace",
        ));
    }
    let bytes = std::fs::read(&resolved).map_err(|e| {
        harness_errors::web_fetch_validation_error(
            "WEB_READ_FAILED",
            format!("web_fetch read failed: {e}"),
            "Confirm the file exists and is readable",
        )
    })?;
    let body = decode_bytes(&bytes, None, None);
    let label = file_tools::display_relative(workspace, &resolved);
    Ok(FetchedContent {
        source_label: label.clone(),
        body,
        content_type: "text/html".into(),
        status_line: format!("path: {label}\n"),
        base_url: None,
        body_text_override: None,
    })
}

fn process_content(
    body: &str,
    content_type: &str,
    base_url: Option<&str>,
    max_links: usize,
) -> Result<PageMeta> {
    let ct = content_type.to_ascii_lowercase();
    if ct.contains("json") {
        let pretty = format_json_body(body);
        return Ok(PageMeta {
            title: None,
            description: None,
            headings: Vec::new(),
            links: Vec::new(),
            body_text: pretty,
        });
    }
    if ct.contains("markdown") || ct.starts_with("text/plain") {
        return Ok(PageMeta {
            title: None,
            description: None,
            headings: Vec::new(),
            links: Vec::new(),
            body_text: body.trim().to_string(),
        });
    }

    let html = extract_main_html(body);
    Ok(PageMeta {
        title: extract_html_title(body),
        description: extract_meta_description(body),
        headings: extract_headings(body),
        links: extract_links(body, base_url, max_links),
        body_text: html_to_text(&html),
    })
}

fn format_json_body(body: &str) -> String {
    match serde_json::from_str::<Value>(body) {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| body.to_string()),
        Err(_) => body.to_string(),
    }
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(800).any(|&b| b == 0)
}

fn decode_bytes(bytes: &[u8], header_charset: Option<&str>, html_sniff: Option<&str>) -> String {
    let charset = header_charset
        .map(str::to_string)
        .or_else(|| html_sniff.and_then(parse_meta_charset));
    if let Some(label) = charset {
        if let Some(enc) = encoding_rs::Encoding::for_label(label.as_bytes()) {
            let (cow, _, _) = enc.decode(bytes);
            return cow.into_owned();
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn parse_charset_from_content_type(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .skip(1)
        .find_map(|part| {
            let part = part.trim();
            part.strip_prefix("charset=")
                .map(|rest| rest.trim().trim_matches('"').to_string())
        })
}

fn parse_meta_charset(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    if let Some(idx) = lower.find("<meta charset=") {
        let rest = &html[idx..];
        if let Some(q) = rest.find('"').and_then(|i| {
            let after = &rest[i + 1..];
            after.find('"').map(|j| &after[..j])
        }) {
            return Some(q.to_string());
        }
    }
    if let Some(idx) = lower.find("http-equiv=\"content-type\"") {
        let snippet = &html[idx..idx.saturating_add(200).min(html.len())];
        return parse_charset_from_content_type(snippet);
    }
    None
}

fn extract_main_html(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    for tag in ["main", "article"] {
        if let Some(inner) = extract_tag_inner(html, &lower, tag) {
            if inner.chars().filter(|c| !c.is_whitespace()).count() > 50 {
                return inner;
            }
        }
    }
    let mut work = html.to_string();
    for tag in ["nav", "footer", "header", "aside"] {
        work = strip_tag_block(&work, tag);
    }
    extract_tag_inner(&work, &work.to_ascii_lowercase(), "body").unwrap_or(work)
}

fn extract_tag_inner(html: &str, lower: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let start = lower.find(&open)?;
    let after_open = &html[start..];
    let gt = after_open.find('>')? + 1;
    let rest = &after_open[gt..];
    let close = format!("</{tag}>");
    let end = rest.to_ascii_lowercase().find(&close)?;
    Some(rest[..end].to_string())
}

fn extract_meta_description(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let idx = lower.find("name=\"description\"").or_else(|| lower.find("name='description'"))?;
    let snippet = &html[idx..idx.saturating_add(300).min(html.len())];
    extract_attr_value(snippet, "content").map(|s| decode_basic_entities(s.trim()))
}

fn extract_attr_value(snippet: &str, attr: &str) -> Option<String> {
    let pattern = format!("{attr}=\"");
    if let Some(i) = snippet.find(&pattern) {
        let rest = &snippet[i + pattern.len()..];
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    let pattern = format!("{attr}='");
    if let Some(i) = snippet.find(&pattern) {
        let rest = &snippet[i + pattern.len()..];
        let end = rest.find('\'')?;
        return Some(rest[..end].to_string());
    }
    None
}

fn extract_headings(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    for level in 1..=3 {
        let open = format!("<h{level}");
        let close = format!("</h{level}>");
        let mut search = 0usize;
        while search < lower.len() {
            let Some(rel) = lower[search..].find(&open) else {
                break;
            };
            let start = search + rel;
            let after = &html[start..];
            let gt = after.find('>').map(|i| i + 1).unwrap_or(0);
            let rest = &after[gt..];
            let Some(end) = rest.to_ascii_lowercase().find(&close) else {
                search = start + open.len();
                continue;
            };
            let text = strip_remaining_tags(&rest[..end]);
            let text = decode_basic_entities(text.trim());
            if !text.is_empty() && out.len() < 30 {
                out.push(text);
            }
            search = start + gt + end + close.len();
        }
    }
    out
}

fn extract_links(html: &str, base_url: Option<&str>, max: usize) -> Vec<String> {
    let re = HREF_RE.get_or_init(|| Regex::new(r#"(?i)href\s*=\s*["']([^"'#]+)["']"#).unwrap());
    let base = base_url.and_then(|u| Url::parse(u).ok());
    let mut seen = HashMap::new();
    let mut out = Vec::new();
    for cap in re.captures_iter(html) {
        let href = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        if href.is_empty() || href.starts_with('#') || href.starts_with("javascript:") {
            continue;
        }
        let resolved = if let Some(ref base) = base {
            base.join(href)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| href.to_string())
        } else {
            href.to_string()
        };
        if seen.insert(resolved.clone(), ()).is_some() {
            continue;
        }
        out.push(resolved);
        if out.len() >= max {
            break;
        }
    }
    out
}

fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let after_tag = &html[start..];
    let content_start = after_tag.find('>')? + 1;
    let rest = &after_tag[content_start..];
    let end = rest.to_ascii_lowercase().find("</title>")?;
    let title = rest[..end].trim();
    if title.is_empty() {
        None
    } else {
        Some(decode_basic_entities(title))
    }
}

fn html_to_text(html: &str) -> String {
    let mut work = html.to_string();
    for tag in ["script", "style", "noscript"] {
        work = strip_tag_block(&work, tag);
    }
    work = replace_block_tags_with_newlines(&work);
    work = strip_remaining_tags(&work);
    let text = decode_basic_entities(&work);
    collapse_blank_lines(&text)
}

fn strip_tag_block(html: &str, tag: &str) -> String {
    let mut out = html.to_string();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    loop {
        let lower = out.to_ascii_lowercase();
        let Some(start) = lower.find(&open) else {
            break;
        };
        let Some(rel_end) = lower[start..].find(&close) else {
            out.replace_range(start.., "");
            break;
        };
        let end = start + rel_end + close.len();
        out.replace_range(start..end, "\n");
    }
    out
}

fn replace_block_tags_with_newlines(html: &str) -> String {
    let block_tags = [
        "p", "div", "br", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "section",
        "article", "header", "footer", "main", "table", "thead", "tbody",
    ];
    let mut out = html.to_string();
    for tag in block_tags {
        for token in [
            format!("<{tag}>"),
            format!("<{tag} "),
            format!("</{tag}>"),
            format!("<{tag}/>"),
        ] {
            out = out.replace(&token, "\n");
        }
        if tag == "br" {
            out = out.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n");
        }
    }
    out
}

fn strip_remaining_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn decode_basic_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::new();
    let mut blank_run = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_run += 1;
            if blank_run <= 2 {
                out.push('\n');
            }
            continue;
        }
        blank_run = 0;
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(trimmed);
        out.push('\n');
    }
    out.trim().to_string()
}

fn web_fetch_ssrf_envelope(host: &str, allow_localhost: bool) -> ErrorEnvelope {
    let hint = if allow_localhost {
        "Private host blocked — only loopback is allowed when allow_localhost is true"
    } else {
        "Set chat.web_fetch.allow_localhost: true in coworker.yaml for localhost dev servers"
    };
    ErrorEnvelope {
        code: "WEB_SSRF_BLOCKED".into(),
        tool_name: WEB_FETCH_TOOL.into(),
        what: "URL host is not allowed".into(),
        why: format!("Host `{host}` resolves to a private or loopback address"),
        try_steps: vec![
            hint.into(),
            "Use a public https URL, or read local HTML via workspace-relative path".into(),
        ],
        example: Some(harness_errors::web_fetch_tool_example(
            "https://example.com",
            "full",
            false,
        )),
        detail: None,
    }
}

fn web_fetch_forbidden_envelope(status: u16, js_challenge: bool) -> ErrorEnvelope {
    let mut try_steps = vec![
        "Do not repeat the same URL — login cookies are not available".into(),
        "Ask the user to open the page or paste the content".into(),
        "For GitHub data use MCP tools (pr_get_*), not web_fetch on github.com".into(),
    ];
    if js_challenge {
        try_steps.insert(
            0,
            "Retry with browser: true on the same URL".into(),
        );
    }
    ErrorEnvelope {
        code: if js_challenge {
            "WEB_ANTI_BOT_JS".into()
        } else {
            "WEB_FORBIDDEN".into()
        },
        tool_name: WEB_FETCH_TOOL.into(),
        what: format!("HTTP {status} — access denied"),
        why: if js_challenge {
            "The server returned a JavaScript anti-bot challenge (e.g. zhihu zse-ck)".into()
        } else {
            "The server rejected the request (auth or permission)".into()
        },
        try_steps,
        example: Some(harness_errors::web_fetch_tool_example(
            "https://www.zhihu.com/question/1",
            "full",
            true,
        )),
        detail: None,
    }
}

fn web_fetch_too_large_envelope(size: usize, max: usize) -> ErrorEnvelope {
    ErrorEnvelope {
        code: "WEB_BODY_TOO_LARGE".into(),
        tool_name: WEB_FETCH_TOOL.into(),
        what: "Response body exceeds download limit".into(),
        why: format!("Got {size} bytes (max {max})"),
        try_steps: vec![
            "Retry with mode=metadata to fetch title/headings/links only".into(),
            "Use bash_run curl with a narrower endpoint".into(),
        ],
        example: Some(harness_errors::web_fetch_tool_example(
            "https://example.com",
            "metadata",
            false,
        )),
        detail: None,
    }
}

fn web_fetch_binary_envelope(content_type: &str) -> ErrorEnvelope {
    ErrorEnvelope {
        code: "WEB_BINARY".into(),
        tool_name: WEB_FETCH_TOOL.into(),
        what: "Response looks binary or unsupported".into(),
        why: format!("content-type: {content_type}"),
        try_steps: vec![
            "Use bash_run curl -I to inspect headers".into(),
            "Download with bash_run curl -o and read_file if it is text".into(),
        ],
        example: Some(harness_errors::web_fetch_tool_example(
            "https://example.com/data.json",
            "full",
            false,
        )),
        detail: None,
    }
}

fn web_fetch_envelope(url: &str, err: &str, status: Option<u16>) -> ErrorEnvelope {
    let low = err.to_ascii_lowercase();
    let (code, what, try_steps) = if low.contains("timed out") || low.contains("timeout") {
        (
            "WEB_TIMEOUT",
            "HTTP request timed out",
            vec![
                "Confirm dev server is running (bash_run curl -I …)".into(),
                "Increase chat.web_fetch.timeout_secs or retry with mode=metadata".into(),
            ],
        )
    } else if status == Some(404) || low.contains("404") {
        (
            "WEB_NOT_FOUND",
            "URL not found (404)",
            vec!["Check the URL path and spelling".into()],
        )
    } else {
        (
            "WEB_FETCH_FAILED",
            "HTTP fetch failed",
            vec![
                "Verify the URL is reachable from this machine".into(),
                "For localhost set chat.web_fetch.allow_localhost: true".into(),
            ],
        )
    };
    ErrorEnvelope {
        code: code.into(),
        tool_name: WEB_FETCH_TOOL.into(),
        what: what.into(),
        why: err.to_string(),
        try_steps,
        example: Some(harness_errors::web_fetch_tool_example(url, "full", false)),
        detail: Some(format!("url: {url}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn html_to_text_strips_tags_and_scripts() {
        let html = "<html><head><title>Hi</title><script>alert(1)</script></head>\
            <body><h1>Title</h1><p>Hello <b>world</b></p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Title"));
        assert!(text.contains("Hello world"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn extract_title_and_description() {
        let html = "<head><title>Foo</title>\
            <meta name=\"description\" content=\"Bar &amp; baz\"></head>";
        assert_eq!(extract_html_title(html).as_deref(), Some("Foo"));
        assert_eq!(extract_meta_description(html).as_deref(), Some("Bar & baz"));
    }

    #[test]
    fn extract_headings_and_links() {
        let html = "<main><h1>One</h1><h2>Two</h2><a href=\"/docs\">Docs</a></main>";
        let headings = extract_headings(html);
        assert!(headings.contains(&"One".to_string()));
        assert!(headings.contains(&"Two".to_string()));
        let links = extract_links(html, Some("https://example.com/page"), 10);
        assert!(links.iter().any(|l| l.contains("/docs")));
    }

    #[test]
    fn main_content_prefers_main_tag() {
        let html = "<nav>Menu</nav><main><p>Core</p></main><footer>Foot</footer>";
        let main = extract_main_html(html);
        assert!(main.contains("Core"));
        assert!(!main.contains("Menu"));
    }

    #[test]
    fn format_json_pretty() {
        let out = format_json_body("{\"a\":1}");
        assert!(out.contains("\"a\""));
        assert!(out.contains('\n'));
    }

    #[test]
    fn decode_gbk_bytes() {
        let bytes: &[u8] = &[0xD6, 0xD0, 0xCE, 0xC4]; // 中文 in GBK
        let text = decode_bytes(bytes, Some("gbk"), None);
        assert_eq!(text, "中文");
    }

    #[test]
    fn normalize_localhost_url() {
        assert_eq!(normalize_url_input("localhost:5173"), "http://localhost:5173");
    }

    #[test]
    fn ssrf_blocks_loopback_by_default() {
        let config = WebFetchToolConfig::default();
        assert!(validate_remote_url("http://127.0.0.1:8080", &config).is_err());
        let allowed = WebFetchToolConfig {
            allow_localhost: true,
            ..Default::default()
        };
        assert!(validate_remote_url("http://127.0.0.1:8080", &allowed).is_ok());
    }

    #[test]
    fn parse_browser_arg_defaults_false() {
        assert!(!parse_browser_arg(&json!({})));
    }

    #[test]
    fn parse_browser_arg_true_when_set() {
        assert!(parse_browser_arg(&json!({ "browser": true })));
        assert!(parse_browser_arg(&json!({ "browser": "yes" })));
    }

    #[tokio::test]
    #[ignore = "manual: needs Chrome + network"]
    async fn chromium_fetch_example_smoke() {
        let config = WebFetchToolConfig::default();
        let args = json!({
            "url": "https://example.com",
            "browser": true,
            "mode": "metadata"
        });
        let dir = TempDir::new().unwrap();
        let out = execute_web_fetch_tool(&config, dir.path(), &args)
            .await
            .unwrap_or_else(|e| panic!("browser fetch failed: {e}"));
        assert!(out.contains("headless-chromium"));
        eprintln!("{out}");
    }

    #[tokio::test]
    #[ignore = "manual: needs Chrome + network"]
    async fn chromium_fetch_zhihu_smoke() {
        let config = WebFetchToolConfig::default();
        let args = json!({
            "url": "https://www.zhihu.com/question/528932203",
            "browser": true,
            "mode": "metadata"
        });
        let dir = TempDir::new().unwrap();
        let out = execute_web_fetch_tool(&config, dir.path(), &args)
            .await
            .unwrap_or_else(|e| panic!("zhihu browser fetch failed: {e}"));
        assert!(out.contains("headless-chromium"));
        assert!(
            !out.contains("403") || out.contains("title:"),
            "expected rendered page metadata, got:\n{out}"
        );
        eprintln!("{out}");
    }

    #[test]
    fn rejects_non_http_schemes() {
        let config = WebFetchToolConfig::default();
        assert!(validate_remote_url("javascript:alert(1)", &config).is_err());
        assert!(validate_remote_url("https://example.com", &config).is_ok());
    }

    #[tokio::test]
    async fn local_html_full_mode() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("page.html"),
            "<html><head><title>Local</title></head><body><p>Preview me</p></body></html>",
        )
        .unwrap();
        let config = WebFetchToolConfig::default();
        let out = execute_web_fetch_tool(&config, dir.path(), &json!({ "url": "page.html" }))
            .await
            .unwrap();
        assert!(out.contains("title: Local"));
        assert!(out.contains("Preview me"));
        assert!(out.contains("---"));
    }

    #[tokio::test]
    async fn metadata_mode_omits_body() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("page.html"),
            "<html><head><title>T</title></head><body><p>Secret body</p></body></html>",
        )
        .unwrap();
        let config = WebFetchToolConfig::default();
        let out = execute_web_fetch_tool(
            &config,
            dir.path(),
            &json!({ "url": "page.html", "mode": "metadata" }),
        )
        .await
        .unwrap();
        assert!(out.contains("title: T"));
        assert!(!out.contains("Secret body"));
        assert!(!out.contains("---"));
    }
}
