use std::sync::Arc;

use tokio::sync::broadcast;

use crate::config::Config;
use crate::store::{
    Approval, AuditEntry, Digest, FlakyTestRollup, LogLine, PrSnapshot, Store,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard = 0,
    Prs = 1,
    Approvals = 2,
    Logs = 3,
    Config = 4,
    Flaky = 5,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Dashboard,
        Tab::Prs,
        Tab::Approvals,
        Tab::Logs,
        Tab::Config,
        Tab::Flaky,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Dashboard => "1 Dashboard",
            Tab::Prs => "2 PRs",
            Tab::Approvals => "3 Approvals",
            Tab::Logs => "4 Logs",
            Tab::Config => "5 Config",
            Tab::Flaky => "6 Flaky",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Tab::Dashboard => Tab::Prs,
            Tab::Prs => Tab::Approvals,
            Tab::Approvals => Tab::Logs,
            Tab::Logs => Tab::Config,
            Tab::Config => Tab::Flaky,
            Tab::Flaky => Tab::Dashboard,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Tab::Dashboard => Tab::Flaky,
            Tab::Prs => Tab::Dashboard,
            Tab::Approvals => Tab::Prs,
            Tab::Logs => Tab::Approvals,
            Tab::Config => Tab::Logs,
            Tab::Flaky => Tab::Config,
        }
    }
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    StoreUpdated,
    DigestReady(Digest),
    LogLine(LogLine),
    WorkflowStarted { workflow_id: String },
    WorkflowFinished {
        workflow_id: String,
        ok: bool,
        message: String,
    },
    StatusMessage(String),
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub config: Config,
    pub config_path: String,
    pub tab: Tab,
    pub latest_digest: Option<Digest>,
    pub prs: Vec<PrSnapshot>,
    pub approvals: Vec<Approval>,
    pub flaky_tests: Vec<FlakyTestRollup>,
    pub logs: Vec<LogLine>,
    pub selected_index: usize,
    pub detail_scroll: u16,
    pub status: String,
    pub engine_busy: bool,
    pub mcp_ok: bool,
    pub llm_ok: bool,
}

impl AppState {
    pub fn new(config: Config, config_path: String) -> Self {
        Self {
            config,
            config_path,
            tab: Tab::Dashboard,
            latest_digest: None,
            prs: vec![],
            approvals: vec![],
            flaky_tests: vec![],
            logs: vec![],
            selected_index: 0,
            detail_scroll: 0,
            status: "ready".into(),
            engine_busy: false,
            mcp_ok: false,
            llm_ok: false,
        }
    }

    pub fn push_log(&mut self, level: &str, message: impl Into<String>) {
        self.logs.push(LogLine {
            ts: chrono::Utc::now(),
            level: level.into(),
            message: message.into(),
        });
        if self.logs.len() > 500 {
            let drain = self.logs.len() - 500;
            self.logs.drain(0..drain);
        }
    }

    pub fn selected_pr(&self) -> Option<&PrSnapshot> {
        self.prs.get(self.selected_index)
    }

    pub fn selected_approval(&self) -> Option<&Approval> {
        self.approvals.get(self.selected_index)
    }
}

pub type SharedState = Arc<tokio::sync::RwLock<AppState>>;

pub fn event_channel() -> (broadcast::Sender<AppEvent>, broadcast::Receiver<AppEvent>) {
    broadcast::channel(256)
}

pub async fn hydrate_from_store(state: &SharedState, store: &dyn Store) -> crate::error::Result<()> {
    let digest = store.latest_digest().await?;
    let prs = store.list_pr_snapshots(None).await?;
    let approvals = store.list_pending_approvals().await?;
    let flaky = store
        .list_flaky_tests(crate::store::FlakyQuery {
            repo: None,
            since_days: Some(30),
            limit: 50,
        })
        .await?;

    let mut s = state.write().await;
    s.latest_digest = digest;
    s.prs = prs;
    s.approvals = approvals;
    s.flaky_tests = flaky;
    Ok(())
}

pub async fn append_audit(store: &dyn Store, level: &str, event: &str, message: &str) {
    let entry = AuditEntry {
        id: uuid::Uuid::new_v4(),
        ts: chrono::Utc::now(),
        level: level.into(),
        event: event.into(),
        message: message.into(),
    };
    let _ = store.append_audit(&entry).await;
}
