//! Optional GitHub Releases version check (no telemetry).

use serde::{Deserialize, Serialize};

const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/unistar-ai/unistar-coworker/releases/latest";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeInfo {
    pub current: String,
    pub latest: Option<String>,
    pub update_available: bool,
    pub release_url: Option<String>,
    pub warning: Option<String>,
}

/// Return true when `latest` is strictly newer than `current` (semver).
pub fn is_newer_version(latest: &str, current: &str) -> bool {
    match (
        semver::Version::parse(latest),
        semver::Version::parse(current),
    ) {
        (Ok(l), Ok(c)) => l > c,
        _ => false,
    }
}

/// Query GitHub Releases for the latest tag. Offline / rate limit → `warning`, exit-friendly.
pub async fn check_upgrade(current: &str) -> UpgradeInfo {
    let mut info = UpgradeInfo {
        current: current.to_string(),
        latest: None,
        update_available: false,
        release_url: None,
        warning: None,
    };

    let client = match reqwest::Client::builder()
        .user_agent(format!("unistar-coworker/{current}"))
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            info.warning = Some(format!("could not build HTTP client: {e}"));
            return info;
        }
    };

    match client.get(RELEASES_LATEST_URL).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(body) => {
                let tag = body
                    .get("tag_name")
                    .and_then(|v| v.as_str())
                    .map(|t| t.trim_start_matches('v').to_string());
                info.latest = tag.clone();
                info.release_url = body
                    .get("html_url")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                if let Some(latest) = &tag {
                    info.update_available = is_newer_version(latest, current);
                }
            }
            Err(e) => {
                info.warning = Some(format!("could not parse GitHub releases JSON: {e}"));
            }
        },
        Ok(resp) => {
            info.warning = Some(format!(
                "GitHub API returned HTTP {} — skipping version compare",
                resp.status()
            ));
        }
        Err(e) => {
            info.warning = Some(format!("could not reach GitHub releases API: {e}"));
        }
    }

    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_compare_detects_update() {
        assert!(is_newer_version("2.1.0", "2.0.1"));
        assert!(!is_newer_version("2.0.1", "2.0.1"));
        assert!(!is_newer_version("2.0.0", "2.0.1"));
    }

    #[test]
    fn semver_compare_ignores_invalid() {
        assert!(!is_newer_version("not-a-version", "2.0.1"));
    }
}
