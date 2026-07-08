//! Shared health checks for `doctor`, Web `/api/doctor`, and status probes.

use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_yaml::Value;

use crate::agent::redact::looks_like_secret;
use crate::config::{is_unresolved_env_placeholder, Config};
use crate::store::open_store;

#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    pub name: &'static str,
    pub status: &'static str, // "ok" | "warn" | "fail"
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub ok: usize,
    pub warn: usize,
    pub fail: usize,
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    pub fn has_failures(&self) -> bool {
        self.fail > 0
    }

    pub fn push_check(&mut self, check: DoctorCheck) {
        self.checks.push(check);
    }

    fn finalize_counts(&mut self) {
        self.fail = self.checks.iter().filter(|c| c.status == "fail").count();
        self.warn = self.checks.iter().filter(|c| c.status == "warn").count();
        self.ok = self.checks.len() - self.fail - self.warn;
    }
}

/// Optional platform checks supplied by the CLI (e.g. web-ui embed status).
#[derive(Debug, Clone, Default)]
pub struct DoctorExtras {
    pub web_ui_detail: Option<String>,
    pub web_ui_status: Option<&'static str>,
}

/// Run all configured health checks (config optional when file missing).
pub async fn run_checks(config_override: Option<PathBuf>) -> DoctorReport {
    run_checks_with_extras(config_override, DoctorExtras::default()).await
}

pub async fn run_checks_with_extras(
    config_override: Option<PathBuf>,
    extras: DoctorExtras,
) -> DoctorReport {
    use std::process::Command;

    let mut report = DoctorReport {
        ok: 0,
        warn: 0,
        fail: 0,
        checks: Vec::new(),
    };

    let loaded = match &config_override {
        Some(p) => Config::load(p),
        None => Config::discover().map(|(c, _)| c),
    };
    let config_path_label = config_override
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "coworker.yaml (cwd or .coworker/)".into());
    let cfg = match &loaded {
        Ok(c) => {
            report.push_check(DoctorCheck {
                name: "config",
                status: "ok",
                detail: format!("loaded {config_path_label}"),
                latency_ms: None,
                hint: None,
            });
            Some(c.clone())
        }
        Err(e) => {
            report.push_check(DoctorCheck {
                name: "config",
                status: "fail",
                detail: e.to_string(),
                latency_ms: None,
                hint: Some("run `unistar-coworker init --interactive` or fix coworker.yaml".into()),
            });
            None
        }
    };

    match Command::new("gh").args(["auth", "status"]).output() {
        Ok(o) if o.status.success() => report.push_check(DoctorCheck {
            name: "github",
            status: "ok",
            detail: "gh authenticated".into(),
            latency_ms: None,
            hint: None,
        }),
        Ok(o) => {
            let msg = String::from_utf8_lossy(&o.stderr);
            let first = msg
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("not authenticated")
                .to_string();
            report.push_check(DoctorCheck {
                name: "github",
                status: "fail",
                detail: first,
                latency_ms: None,
                hint: Some("run `gh auth login`".into()),
            });
        }
        Err(e) => report.push_check(DoctorCheck {
            name: "github",
            status: "fail",
            detail: format!("gh CLI not found: {e}"),
            latency_ms: None,
            hint: Some("install GitHub CLI: https://cli.github.com".into()),
        }),
    }

    if let Some(cfg) = cfg {
        push_config_security_checks(&mut report, &cfg);
        push_storage_writable_check(&mut report, &cfg);
        push_port_check(&mut report, &cfg.web.bind);

        let online = crate::llm::ollama::probe(&cfg.llm).await;
        let latency = crate::llm::ollama::probe_latency_ms(&cfg.llm).await;
        if online {
            let lat = latency
                .map(|l| format!("{l}ms"))
                .unwrap_or_else(|| "n/a".into());
            report.push_check(DoctorCheck {
                name: "llm",
                status: "ok",
                detail: format!("{} reachable ({lat})", cfg.llm.base_url),
                latency_ms: latency,
                hint: None,
            });
        } else {
            report.push_check(DoctorCheck {
                name: "llm",
                status: "fail",
                detail: format!("{} unreachable", cfg.llm.base_url),
                latency_ms: latency,
                hint: Some("check llm.base_url and that the server is running".into()),
            });
        }

        if cfg.mcp.servers.is_empty() {
            report.push_check(DoctorCheck {
                name: "mcp",
                status: "ok",
                detail: "no servers configured".into(),
                latency_ms: None,
                hint: None,
            });
        } else {
            let pool = crate::mcp::McpPool::new(cfg.mcp.clone());
            pool.connect_eager().await;
            for s in pool.status_snapshot().await {
                if s.connected {
                    report.push_check(DoctorCheck {
                        name: "mcp",
                        status: "ok",
                        detail: format!("{} connected ({} tools)", s.id, s.tool_count),
                        latency_ms: s.last_rpc_ms,
                        hint: None,
                    });
                } else {
                    let err = s
                        .last_error
                        .clone()
                        .unwrap_or_else(|| "not connected".into());
                    report.push_check(DoctorCheck {
                        name: "mcp",
                        status: "fail",
                        detail: format!("{}: {err}", s.id),
                        latency_ms: s.last_rpc_ms,
                        hint: Some("verify mcp.servers[] command/URL in coworker.yaml".into()),
                    });
                }
            }
        }

        match open_store(&cfg) {
            Ok(store) => match store.list_chat_sessions(1).await {
                Ok(_) => report.push_check(DoctorCheck {
                    name: "store",
                    status: "ok",
                    detail: format!("{:?} backend readable", cfg.storage.backend),
                    latency_ms: None,
                    hint: None,
                }),
                Err(e) => report.push_check(DoctorCheck {
                    name: "store",
                    status: "fail",
                    detail: e.to_string(),
                    latency_ms: None,
                    hint: None,
                }),
            },
            Err(e) => report.push_check(DoctorCheck {
                name: "store",
                status: "fail",
                detail: e.to_string(),
                latency_ms: None,
                hint: None,
            }),
        }
    } else {
        for name in ["llm", "mcp", "store"] {
            report.push_check(DoctorCheck {
                name,
                status: "fail",
                detail: "skipped: config not loaded".into(),
                latency_ms: None,
                hint: None,
            });
        }
    }

    if let (Some(status), Some(detail)) = (extras.web_ui_status, extras.web_ui_detail) {
        report.push_check(DoctorCheck {
            name: "web-ui",
            status,
            detail,
            latency_ms: None,
            hint: if status == "warn" {
                Some(
                    "build with `--features embed-web-ui` or run `cd web-ui && npm run build:fast`"
                        .into(),
                )
            } else {
                None
            },
        });
    }

    report.finalize_counts();
    report
}

fn push_config_security_checks(report: &mut DoctorReport, cfg: &Config) {
    if cfg.web.bind.contains("0.0.0.0") && !cfg.web.auth_enabled() {
        report.push_check(DoctorCheck {
            name: "web",
            status: "warn",
            detail: format!("web.bind is {} without web.auth_token", cfg.web.bind),
            latency_ms: None,
            hint: Some("set web.auth_token when binding to 0.0.0.0".into()),
        });
    }

    for (name, profile) in &cfg.llm_profiles {
        let Some(key) = profile.api_key.as_deref() else {
            continue;
        };
        let trimmed = key.trim();
        if trimmed.is_empty() {
            continue;
        }
        if is_unresolved_env_placeholder(trimmed) {
            report.push_check(DoctorCheck {
                name: "secrets",
                status: "warn",
                detail: format!("llm.{name}.api_key unresolved placeholder `{trimmed}`"),
                latency_ms: None,
                hint: Some(format!(
                    "export {} or set the env var before serve",
                    trimmed.trim_start_matches("${").trim_end_matches('}')
                )),
            });
        } else if looks_like_secret(trimmed) {
            report.push_check(DoctorCheck {
                name: "secrets",
                status: "fail",
                detail: format!(
                    "llm.{name}.api_key looks like a plaintext secret — use ${{ENV_VAR}} instead"
                ),
                latency_ms: None,
                hint: Some(
                    "move the key to an environment variable and rotate the exposed credential"
                        .into(),
                ),
            });
        }
    }
}

fn push_storage_writable_check(report: &mut DoctorReport, cfg: &Config) {
    let path = cfg.storage_path();
    match storage_writable(&path) {
        Ok(()) => report.push_check(DoctorCheck {
            name: "data-dir",
            status: "ok",
            detail: format!("{} writable", path.display()),
            latency_ms: None,
            hint: None,
        }),
        Err(e) => report.push_check(DoctorCheck {
            name: "data-dir",
            status: "fail",
            detail: format!("{} not writable: {e}", path.display()),
            latency_ms: None,
            hint: Some("check permissions or storage.path in coworker.yaml".into()),
        }),
    }
}

fn storage_writable(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    let probe = path.join(".doctor-write-test");
    std::fs::write(&probe, b"x")?;
    std::fs::remove_file(probe)?;
    Ok(())
}

fn push_port_check(report: &mut DoctorReport, bind: &str) {
    let port = bind_port(bind).unwrap_or(8787);
    let addr: SocketAddr = format!("127.0.0.1:{port}")
        .parse()
        .unwrap_or_else(|_| "127.0.0.1:8787".parse().expect("valid fallback addr"));
    match TcpListener::bind(addr) {
        Ok(_) => report.push_check(DoctorCheck {
            name: "port",
            status: "ok",
            detail: format!("port {port} available on 127.0.0.1"),
            latency_ms: None,
            hint: None,
        }),
        Err(e) => report.push_check(DoctorCheck {
            name: "port",
            status: "warn",
            detail: format!("port {port} in use on 127.0.0.1: {e}"),
            latency_ms: None,
            hint: Some("another unistar-coworker or service may already be listening".into()),
        }),
    }
}

fn bind_port(bind: &str) -> Option<u16> {
    bind.rsplit(':').next()?.parse().ok()
}

/// Redact `api_key` fields in raw coworker.yaml for diagnostic bundles.
pub fn redact_coworker_yaml(raw: &str) -> String {
    let Ok(mut value) = serde_yaml::from_str::<Value>(raw) else {
        return raw.to_string();
    };
    redact_yaml_secrets(&mut value);
    serde_yaml::to_string(&value).unwrap_or_else(|_| raw.to_string())
}

fn redact_yaml_secrets(value: &mut Value) {
    match value {
        Value::Mapping(map) => {
            for (k, v) in map.iter_mut() {
                let key = k.as_str().unwrap_or("");
                if key == "api_key" {
                    *v = Value::String("***redacted***".into());
                } else {
                    redact_yaml_secrets(v);
                }
            }
        }
        Value::Sequence(seq) => {
            for item in seq {
                redact_yaml_secrets(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_coworker_yaml_masks_api_keys() {
        let raw = r#"
llm:
  remote:
    base_url: https://api.example.com/v1
    model: m
    context_limit: 64000
    api_key: sk-live-secret
repos: [acme/widget]
"#;
        let out = redact_coworker_yaml(raw);
        assert!(out.contains("***redacted***"));
        assert!(!out.contains("sk-live-secret"));
    }

    #[test]
    fn bind_port_parses_host_port() {
        assert_eq!(bind_port("127.0.0.1:8787"), Some(8787));
        assert_eq!(bind_port("0.0.0.0:9999"), Some(9999));
    }
}
