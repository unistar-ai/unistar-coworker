use crate::config::LlmConfig;

pub async fn probe(cfg: &LlmConfig) -> bool {
    let base = cfg.base_url.trim_end_matches('/');
    // OpenAI-compatible base (e.g. http://localhost:11434/v1) → /v1/models
    let url = format!("{base}/models");
    match reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => true,
        Ok(_) | Err(_) => {
            // Fallback: native Ollama API
            let native = format!(
                "{}/api/tags",
                base.trim_end_matches("/v1").trim_end_matches('/')
            );
            match reqwest::Client::new()
                .get(&native)
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await
            {
                Ok(r) => r.status().is_success(),
                Err(_) => false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_url_openai_compat() {
        let cfg = LlmConfig {
            base_url: "http://localhost:11434/v1".into(),
            model: "m".into(),
            context_limit: 64000,
            log_page_lines: 80,
            max_log_pages: 8,
            concurrency: 2,
            structured_output: true,
        };
        let base = cfg.base_url.trim_end_matches('/');
        assert_eq!(format!("{base}/models"), "http://localhost:11434/v1/models");
    }
}
