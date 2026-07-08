use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use cron::Schedule;

use crate::config::Config;
use crate::engine::Engine;

struct ScheduledJob {
    workflow_id: String,
    schedule: Schedule,
    label: String,
}

fn normalize_cron(expr: &str) -> String {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() == 5 {
        format!("0 {expr}")
    } else {
        expr.to_string()
    }
}

pub struct Scheduler {
    jobs: Vec<ScheduledJob>,
}

impl Scheduler {
    pub fn from_config(config: &Config) -> Self {
        let mut jobs = Vec::new();
        let mut seen = HashSet::new();

        let mut push = |workflow_id: &str, cron_expr: &str, label: &str| {
            if !config
                .workflows
                .get(workflow_id)
                .map(|w| w.enabled)
                .unwrap_or(false)
            {
                return;
            }
            let key = format!("{workflow_id}:{cron_expr}");
            if !seen.insert(key) {
                return;
            }
            match Schedule::from_str(&normalize_cron(cron_expr)) {
                Ok(schedule) => jobs.push(ScheduledJob {
                    workflow_id: workflow_id.to_string(),
                    schedule,
                    label: label.to_string(),
                }),
                Err(e) => tracing::warn!("invalid cron `{cron_expr}` for {workflow_id}: {e}"),
            }
        };

        if let Some(ref c) = config.schedule.daily_digest {
            push("daily-work", c, "daily_digest");
        }
        if let Some(ref c) = config.schedule.ci_rescan {
            push("daily-work", c, "ci_rescan");
        }

        for (id, wf) in config.workflows.iter() {
            if wf.enabled {
                if let Some(ref c) = wf.schedule {
                    push(id, c, "workflow");
                }
            }
        }

        Self { jobs }
    }

    pub fn spawn(self, engine: Arc<Engine>) {
        if self.jobs.is_empty() {
            engine.emit_log("info", "scheduler: no cron jobs configured");
            return;
        }

        for job in self.jobs {
            let engine = Arc::clone(&engine);
            tokio::spawn(async move {
                engine.emit_log(
                    "info",
                    format!(
                        "scheduler: {} → {} ({})",
                        job.label, job.workflow_id, job.schedule
                    ),
                );
                loop {
                    let now = Utc::now();
                    let sleep_for = job
                        .schedule
                        .upcoming(Utc)
                        .next()
                        .map(|next| {
                            next.signed_duration_since(now)
                                .to_std()
                                .unwrap_or(Duration::from_secs(60))
                        })
                        .unwrap_or(Duration::from_secs(3600));

                    tokio::time::sleep(sleep_for).await;

                    if engine.is_busy().await {
                        engine.emit_log(
                            "warn",
                            format!("scheduler: skip {} — engine busy", job.workflow_id),
                        );
                        continue;
                    }

                    engine.emit_log("info", format!("scheduler: running {}", job.workflow_id));
                    if let Err(e) = engine.run_workflow(&job.workflow_id).await {
                        engine.emit_log(
                            "warn",
                            format!("scheduler: {} failed: {e}", job.workflow_id),
                        );
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn normalizes_five_field_cron() {
        assert_eq!(normalize_cron("0 6 * * *"), "0 0 6 * * *");
    }

    #[test]
    fn picks_enabled_workflow_schedules() {
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
github: { gh_command: gh }
storage: { backend: json, path: ./data }
repos: [org/repo]
workflows:
  daily-work:
    enabled: true
  review-radar:
    enabled: true
    schedule: "0 9 * * 1-5"
schedule:
  daily_digest: "0 6 * * *"
"#;
        let config = Config::load_from_str(yaml).unwrap();
        let sched = Scheduler::from_config(&config);
        assert!(!sched.jobs.is_empty());
        assert!(
            sched.jobs.iter().any(|j| j.workflow_id == "daily-work"),
            "expected daily-work schedule"
        );
    }
}
