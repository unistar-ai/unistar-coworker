//! Shared health checks for `doctor`, Web `/api/doctor`, and status probes.

use std::path::PathBuf;

use serde::Serialize;

use crate::config::Config;
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
}

/// Run all configured health checks (config optional when file missing).
pub async fn run_checks(config_override: Option<PathBuf>) -> DoctorReport {
    use std::process::Command;

    let mut checks: Vec<DoctorCheck> = Vec::new();

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
            checks.push(DoctorCheck {
                name: "config",
                status: "ok",
                detail: format!("loaded {config_path_label}"),
                latency_ms: None,
                hint: None,
            });
            Some(c.clone())
        }
        Err(e) => {
            checks.push(DoctorCheck {
                name: "config",
                status: "fail",
                detail: e.to_string(),
                latency_ms: None,
                hint: Some("run `unistar-coworker init` or fix coworker.yaml".into()),
            });
            None
        }
    };

    match Command::new("gh").args(["auth", "status"]).output() {
        Ok(o) if o.status.success() => checks.push(DoctorCheck {
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
            checks.push(DoctorCheck {
                name: "github",
                status: "fail",
                detail: first,
                latency_ms: None,
                hint: Some("run `gh auth login`".into()),
            });
        }
        Err(e) => checks.push(DoctorCheck {
            name: "github",
            status: "fail",
            detail: format!("gh CLI not found: {e}"),
            latency_ms: None,
            hint: Some("install GitHub CLI: https://cli.github.com".into()),
        }),
    }

    if let Some(cfg) = cfg {
        let online = crate::llm::ollama::probe(&cfg.llm).await;
        let latency = crate::llm::ollama::probe_latency_ms(&cfg.llm).await;
        if online {
            let lat = latency
                .map(|l| format!("{l}ms"))
                .unwrap_or_else(|| "n/a".into());
            checks.push(DoctorCheck {
                name: "llm",
                status: "ok",
                detail: format!("{} reachable ({lat})", cfg.llm.base_url),
                latency_ms: latency,
                hint: None,
            });
        } else {
            checks.push(DoctorCheck {
                name: "llm",
                status: "fail",
                detail: format!("{} unreachable", cfg.llm.base_url),
                latency_ms: latency,
                hint: Some("check llm.base_url and that the server is running".into()),
            });
        }

        if cfg.mcp.servers.is_empty() {
            checks.push(DoctorCheck {
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
                    checks.push(DoctorCheck {
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
                    checks.push(DoctorCheck {
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
                Ok(_) => checks.push(DoctorCheck {
                    name: "store",
                    status: "ok",
                    detail: format!("{:?} backend readable", cfg.storage.backend),
                    latency_ms: None,
                    hint: None,
                }),
                Err(e) => checks.push(DoctorCheck {
                    name: "store",
                    status: "fail",
                    detail: e.to_string(),
                    latency_ms: None,
                    hint: None,
                }),
            },
            Err(e) => checks.push(DoctorCheck {
                name: "store",
                status: "fail",
                detail: e.to_string(),
                latency_ms: None,
                hint: None,
            }),
        }
    } else {
        for name in ["llm", "mcp", "store"] {
            checks.push(DoctorCheck {
                name,
                status: "fail",
                detail: "skipped: config not loaded".into(),
                latency_ms: None,
                hint: None,
            });
        }
    }

    let fail = checks.iter().filter(|c| c.status == "fail").count();
    let warn = checks.iter().filter(|c| c.status == "warn").count();
    let ok = checks.len() - fail - warn;
    DoctorReport {
        ok,
        warn,
        fail,
        checks,
    }
}
