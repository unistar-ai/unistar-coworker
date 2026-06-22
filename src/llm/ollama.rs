use crate::config::LlmConfig;

pub async fn probe(cfg: &LlmConfig) -> bool {
    probe_latency_ms(cfg).await.is_some()
}

/// Round-trip latency for the LLM health check (`/models` or `/api/tags`).
pub async fn probe_latency_ms(cfg: &LlmConfig) -> Option<u128> {
    let base = cfg.base_url.trim_end_matches('/');
    let url = format!("{base}/models");
    let start = std::time::Instant::now();
    match reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => Some(start.elapsed().as_millis()),
        Ok(_) | Err(_) => {
            let native = format!(
                "{}/api/tags",
                base.trim_end_matches("/v1").trim_end_matches('/')
            );
            let start = std::time::Instant::now();
            match reqwest::Client::new()
                .get(&native)
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => Some(start.elapsed().as_millis()),
                _ => None,
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
            max_output_tokens: 4096,
            think: true,
            max_thinking_tokens: 512,
            reasoning_summary_tokens: 320,
            history_summary_tokens: 256,
        };
        let base = cfg.base_url.trim_end_matches('/');
        assert_eq!(format!("{base}/models"), "http://localhost:11434/v1/models");
    }
}
