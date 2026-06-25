use reqwest::RequestBuilder;

use crate::config::LlmConfig;

pub async fn probe(cfg: &LlmConfig) -> bool {
    probe_latency_ms(cfg).await.is_some()
}

/// Round-trip latency for the LLM health check.
pub async fn probe_latency_ms(cfg: &LlmConfig) -> Option<u128> {
    let client = reqwest::Client::new();
    let timeout = std::time::Duration::from_secs(5);
    let root = server_root(&cfg.base_url);

    // oMLX / generic OpenAI servers often expose unauthenticated /health.
    if let Some(ms) = probe_get(&client, format!("{root}/health"), timeout, None).await {
        return Some(ms);
    }

    let models_url = format!("{}/models", openai_v1_base(&cfg.base_url));
    if let Some(ms) = probe_get(&client, models_url, timeout, Some(cfg)).await {
        return Some(ms);
    }

    if is_ollama_base_url(&cfg.base_url) {
        let tags_url = format!("{root}/api/tags");
        if let Some(ms) = probe_get(&client, tags_url, timeout, None).await {
            return Some(ms);
        }
    }

    None
}

/// Host root without a trailing `/v1` (e.g. `http://localhost:12345`).
pub fn server_root(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    trimmed
        .strip_suffix("/v1")
        .unwrap_or(trimmed)
        .trim_end_matches('/')
        .to_string()
}

/// OpenAI-compatible API prefix (`…/v1`).
pub fn openai_v1_base(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

/// True when the URL should use Ollama's native `/api/chat` (not OpenAI `/v1/chat/completions`).
pub fn is_ollama_base_url(base_url: &str) -> bool {
    let root = server_root(base_url).to_ascii_lowercase();
    root.contains("11434") || root.contains("ollama")
}

/// Base URL for Ollama native API (`/api/chat`), if this host is Ollama.
pub fn ollama_native_base(base_url: &str) -> Option<String> {
    if !is_ollama_base_url(base_url) {
        return None;
    }
    Some(server_root(base_url))
}

pub fn apply_llm_auth(builder: RequestBuilder, cfg: &LlmConfig) -> RequestBuilder {
    if let Some(key) = cfg
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        builder.bearer_auth(key)
    } else {
        builder
    }
}

async fn probe_get(
    client: &reqwest::Client,
    url: String,
    timeout: std::time::Duration,
    cfg: Option<&LlmConfig>,
) -> Option<u128> {
    let start = std::time::Instant::now();
    let mut req = client.get(&url).timeout(timeout);
    if let Some(cfg) = cfg {
        req = apply_llm_auth(req, cfg);
    }
    let resp = req.send().await.ok()?;
    let status = resp.status();
    if status.is_success() {
        return Some(start.elapsed().as_millis());
    }
    // Reachable LLM Provider server that requires auth.
    if cfg.is_none() && (status.as_u16() == 401 || status.as_u16() == 403) {
        tracing::debug!("llm probe {url}: HTTP {status} (server up, API key may be required)");
        return Some(start.elapsed().as_millis());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cfg(base_url: &str) -> LlmConfig {
        LlmConfig {
            base_url: base_url.into(),
            model: "m".into(),
            context_limit: 64000,
            log_page_lines: 80,
            max_log_pages: 8,
            concurrency: 2,
            structured_output: true,
            max_output_tokens: 4096,
            think: true,
            max_thinking_tokens: 512,
            reasoning_summary_tokens: 320,
            history_summary_tokens: 256,
            api_key: None,
        }
    }

    #[test]
    fn openai_v1_base_appends_suffix() {
        assert_eq!(
            openai_v1_base("http://localhost:12345"),
            "http://localhost:12345/v1"
        );
        assert_eq!(
            openai_v1_base("http://localhost:12345/v1"),
            "http://localhost:12345/v1"
        );
    }

    #[test]
    fn server_root_strips_v1() {
        assert_eq!(
            server_root("http://localhost:12345/v1"),
            "http://localhost:12345"
        );
    }

    #[test]
    fn omlx_is_not_ollama_native() {
        assert!(!is_ollama_base_url("http://localhost:12345/v1"));
        assert!(ollama_native_base("http://localhost:12345/v1").is_none());
    }

    #[test]
    fn ollama_default_port_uses_native_api() {
        assert!(is_ollama_base_url("http://localhost:11434/v1"));
        assert_eq!(
            ollama_native_base("http://localhost:11434/v1").as_deref(),
            Some("http://localhost:11434")
        );
    }

    #[test]
    fn probe_models_url_openai_compat() {
        let cfg = sample_cfg("http://localhost:11434/v1");
        assert_eq!(
            format!("{}/models", openai_v1_base(&cfg.base_url)),
            "http://localhost:11434/v1/models"
        );
    }
}
