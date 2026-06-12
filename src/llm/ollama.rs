use crate::config::LlmConfig;

pub async fn probe(cfg: &LlmConfig) -> bool {
    let url = format!("{}/models", cfg.base_url.trim_end_matches("/v1"));
    match reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    {
        Ok(r) => r.status().is_success(),
        Err(_) => false,
    }
}
