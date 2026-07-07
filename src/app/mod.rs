mod approval;
mod events;

pub use approval::spawn_approval_decision;
pub use events::apply_event;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use tokio::sync::broadcast;

use crate::agent::budget::TokenBudget;
use crate::agent::chat_loop::{
    build_context_snapshot, ChatActivityFlow, ChatProgress, ContextSnapshot,
};
use crate::agent::context::chat_message_to_llm;
use crate::agent::tool_catalog::ToolCatalog;
use crate::config::Config;
use crate::store::{
    Approval, AuditEntry, ChatMessage, ChatRole, Digest, DigestMeta, LogLine, PrSnapshot, Store,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrFilter {
    All,
    ReviewBlocked,
    CiFailing,
    MyPrs,
}

impl PrFilter {
    pub fn label(self) -> &'static str {
        match self {
            PrFilter::All => "all",
            PrFilter::ReviewBlocked => "review-blocked",
            PrFilter::CiFailing => "CI failing",
            PrFilter::MyPrs => "my PRs",
        }
    }

    pub fn next(self) -> Self {
        match self {
            PrFilter::All => PrFilter::ReviewBlocked,
            PrFilter::ReviewBlocked => PrFilter::CiFailing,
            PrFilter::CiFailing => PrFilter::MyPrs,
            PrFilter::MyPrs => PrFilter::All,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrSort {
    Updated,
    CiStatus,
    Number,
}

impl PrSort {
    pub fn label(self) -> &'static str {
        match self {
            PrSort::Updated => "updated",
            PrSort::CiStatus => "CI status",
            PrSort::Number => "PR #",
        }
    }

    pub fn next(self) -> Self {
        match self {
            PrSort::Updated => PrSort::CiStatus,
            PrSort::CiStatus => PrSort::Number,
            PrSort::Number => PrSort::Updated,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFilter {
    All,
    Error,
    Warn,
    Info,
}

impl LogFilter {
    pub fn label(self) -> &'static str {
        match self {
            LogFilter::All => "all",
            LogFilter::Error => "error",
            LogFilter::Warn => "warn",
            LogFilter::Info => "info",
        }
    }

    pub fn next(self) -> Self {
        match self {
            LogFilter::All => LogFilter::Error,
            LogFilter::Error => LogFilter::Warn,
            LogFilter::Warn => LogFilter::Info,
            LogFilter::Info => LogFilter::All,
        }
    }

    pub fn matches(self, level: &str) -> bool {
        if self == LogFilter::All {
            return true;
        }
        let lower = level.to_ascii_lowercase();
        match self {
            LogFilter::Error => lower == "error",
            LogFilter::Warn => lower == "warn" || lower == "warning",
            LogFilter::Info => lower == "info",
            LogFilter::All => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Chat = 0,
    Dashboard = 1,
    Prs = 2,
    Approvals = 3,
    Logs = 4,
    Config = 5,
}

impl Tab {
    pub fn all_for_config(config: &Config) -> Vec<Tab> {
        let mut tabs = Vec::new();
        if config.chat.enabled {
            tabs.push(Tab::Chat);
        }
        tabs.extend([
            Tab::Dashboard,
            Tab::Prs,
            Tab::Approvals,
            Tab::Logs,
            Tab::Config,
        ]);
        tabs
    }

    pub fn label(self) -> &'static str {
        match self {
            Tab::Chat => "0 Chat",
            Tab::Dashboard => "1 Dashboard",
            Tab::Prs => "2 PRs",
            Tab::Approvals => "3 Approvals",
            Tab::Logs => "4 Logs",
            Tab::Config => "5 Config",
        }
    }

    pub fn next(self, config: &Config) -> Self {
        let tabs = Self::all_for_config(config);
        let idx = tabs.iter().position(|t| *t == self).unwrap_or(0);
        tabs[(idx + 1) % tabs.len()]
    }

    pub fn prev(self, config: &Config) -> Self {
        let tabs = Self::all_for_config(config);
        let idx = tabs.iter().position(|t| *t == self).unwrap_or(0);
        tabs[(idx + tabs.len() - 1) % tabs.len()]
    }
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    StoreUpdated,
    DigestReady(Digest),
    LogLine(LogLine),
    WorkflowStarted {
        workflow_id: String,
    },
    WorkflowFinished {
        workflow_id: String,
        ok: bool,
        message: String,
    },
    StatusMessage(String),
    ChatReply,
    ChatProgress(ChatProgress),
    /// PR tab detail: `pr_get_overview` body loaded asynchronously.
    PrOverviewReady {
        repo: String,
        pr_number: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChatPaneFocus {
    #[default]
    Messages,
    Context,
}

#[derive(Debug, Clone)]
pub struct ChatPendingApproval {
    pub id: uuid::Uuid,
    pub session_id: uuid::Uuid,
    pub tool_name: String,
    pub tool_args_json: String,
    pub line_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDialogChoice {
    Approve,
    Deny,
}

/// Approve on the modal is blocked briefly after open to prevent mis-clicks.
pub const APPROVAL_ARM_DELAY: Duration = Duration::from_millis(200);

#[derive(Debug, Clone)]
pub struct ApprovalDialog {
    pub id: uuid::Uuid,
    pub tool_name: String,
    pub description: String,
    /// Serialized tool arguments (or approval payload in `comment_body`).
    pub tool_args_json: Option<String>,
    pub choice: ApprovalDialogChoice,
    /// True while an approve/deny request is in flight (blocks duplicate submits).
    pub deciding: bool,
    pub opened_at: Instant,
}

impl ApprovalDialog {
    pub fn approve_armed(&self) -> bool {
        self.opened_at.elapsed() >= APPROVAL_ARM_DELAY
    }

    pub fn approve_arm_ms_remaining(&self) -> u128 {
        APPROVAL_ARM_DELAY
            .as_millis()
            .saturating_sub(self.opened_at.elapsed().as_millis())
    }
}

/// Collapsed Dashboard list sections (`true` = items hidden, header only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DashboardFoldState {
    pub digests: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashboardSection {
    Digests,
}

impl DashboardSection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Digests => "digests",
        }
    }
}

impl DashboardFoldState {
    pub fn is_collapsed(self, section: DashboardSection) -> bool {
        match section {
            DashboardSection::Digests => self.digests,
        }
    }
}

fn default_folded_digest_sections() -> std::collections::HashSet<String> {
    ["Ignorable", "Clear", "Waiting for review", "Notes"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub config: Config,
    pub config_path: String,
    pub tab: Tab,
    pub latest_digest: Option<Digest>,
    pub digest_history: Vec<DigestMeta>,
    pub prs: Vec<PrSnapshot>,
    pub approvals: Vec<Approval>,
    pub pr_filter: PrFilter,
    pub pr_sort: PrSort,
    pub log_filter: LogFilter,
    pub logs: Vec<LogLine>,
    /// Digest markdown bodies keyed by date (for Dashboard detail).
    pub digest_bodies: HashMap<NaiveDate, String>,
    pub selected_index: usize,
    pub status: String,
    pub engine_busy: bool,
    /// Active workflow id while `engine_busy` (shown in status bar).
    pub engine_workflow_id: Option<String>,
    pub github_ok: bool,
    pub llm_ok: bool,
    /// Last MCP `tool_list` round-trip (ms), measured at engine start.
    pub github_latency_ms: Option<u128>,
    pub llm_latency_ms: Option<u128>,
    /// Federated MCP server status (from `mcp.servers[]`).
    pub mcp_servers: Vec<crate::mcp::McpServerStatus>,
    pub chat_input: String,
    pub chat_input_history: Vec<String>,
    /// Index into `chat_input_history` while browsing; `None` = drafting new input.
    pub chat_history_pos: Option<usize>,
    pub chat_lines: Vec<String>,
    /// Tool output keyed by index in `chat_lines` (for expand-on-o).
    pub chat_tool_outputs: std::collections::HashMap<usize, String>,
    /// Raw (uncompressed) reasoning trace keyed by index in `chat_lines`.
    /// Populated when LLM reasoning compression was applied; the summary is
    /// in `chat_tool_outputs` at the same index. Absent when no compression.
    pub chat_reasoning_originals: std::collections::HashMap<usize, String>,
    /// Assistant message UUIDs keyed by `chat_lines` index (branch regenerate).
    pub chat_assistant_ids: std::collections::HashMap<usize, uuid::Uuid>,
    pub chat_expanded_tool_lines: std::collections::HashSet<usize>,
    pub chat_busy: bool,
    /// Partial assistant reply while LLM is streaming (shown in the tail status area).
    pub chat_streaming: Option<String>,
    /// In-progress tool JSON while the model streams `action:tool`.
    pub chat_tool_pending: Option<String>,
    /// MCP tool in flight between ToolStart and ToolDone.
    pub chat_tool_running: Option<String>,
    /// Elapsed / paging hint while `chat_tool_running` is active.
    pub chat_tool_running_detail: Option<String>,
    /// Ollama thinking stream (internal reasoning — not shown as the final answer).
    pub chat_reasoning: Option<String>,
    /// Transient skill / MCP activity (live Messages tail — cleared when step ends).
    pub chat_activity_flow: Option<ChatActivityFlow>,
    /// True while the harness summarizes streamed thinking before the next JSON step.
    pub chat_reasoning_compressing: bool,
    /// Bumped when chat content changes; drives render cache invalidation.
    pub chat_render_revision: u64,
    /// Bumped when transcript lines / tool outputs change (not live streaming fields).
    pub chat_history_revision: u64,
    /// Lines to keep visible above the bottom; 0 = pinned to latest.
    pub chat_scroll_from_bottom: u16,
    pub chat_session_id: Option<uuid::Uuid>,
    /// Inline approval from the current chat turn — resolved with y/n on Chat tab.
    pub chat_pending_approval: Option<ChatPendingApproval>,
    /// Centered approval popup (blocks other keys until decided).
    pub approval_dialog: Option<ApprovalDialog>,
    /// Prevents duplicate approve/deny from rapid clicks or Enter repeats.
    pub approval_decision_in_flight: Option<uuid::Uuid>,
    /// Tracks pending approval count to detect new workflow approvals.
    pub last_pending_approval_count: usize,
    /// Toggle with `\` on Chat tab — shows live LLM context from the agent loop.
    pub chat_context_visible: bool,
    pub chat_context: Option<ContextSnapshot>,
    pub chat_context_scroll_from_bottom: u16,
    pub chat_context_revision: u64,
    /// Which split pane receives ↑/↓ when context panel is open.
    pub chat_pane_focus: ChatPaneFocus,
    /// Vertical scroll offset (wrapped lines) for the Detail pane on non-Chat tabs.
    pub detail_scroll_line: u16,
    /// Inclusive line range for mouse drag-to-copy in Detail (`lo`, `hi`).
    pub detail_select: Option<(u16, u16)>,
    pub detail_selecting: bool,
    /// Dashboard list section collapse (alerts / security / digests).
    pub dashboard_fold: DashboardFoldState,
    /// `##` section titles folded in digest Detail markdown (`z` / `Z` on Dashboard).
    pub digest_folded_sections: std::collections::HashSet<String>,
    /// `pr_get_overview` bodies keyed by `repo#number` for the PRs tab Detail pane.
    pub pr_overview_cache: HashMap<String, String>,
    /// In-flight overview fetch (`repo#number`).
    pub pr_overview_fetching: Option<String>,
    /// TUI started with `--attach` (poll shared store from daemon).
    pub attach_mode: bool,
    /// PR cursor within the selected Dashboard digest (`n` / `N`).
    pub dashboard_digest_pr_index: usize,
    /// Digest list index the PR cursor applies to.
    pub dashboard_digest_pr_digest_idx: Option<usize>,
}

impl AppState {
    pub fn new(config: Config, config_path: String) -> Self {
        Self {
            config_path,
            tab: if config.chat.enabled {
                Tab::Chat
            } else {
                Tab::Dashboard
            },
            config,
            latest_digest: None,
            digest_history: vec![],
            prs: vec![],
            approvals: vec![],
            pr_filter: PrFilter::All,
            pr_sort: PrSort::Updated,
            log_filter: LogFilter::All,
            logs: vec![],
            digest_bodies: HashMap::new(),
            selected_index: 0,
            status: "ready".into(),
            engine_busy: false,
            engine_workflow_id: None,
            github_ok: false,
            llm_ok: false,
            github_latency_ms: None,
            llm_latency_ms: None,
            mcp_servers: Vec::new(),
            chat_input: String::new(),
            chat_input_history: vec![],
            chat_history_pos: None,
            chat_lines: vec![],
            chat_tool_outputs: std::collections::HashMap::new(),
            chat_reasoning_originals: std::collections::HashMap::new(),
            chat_assistant_ids: std::collections::HashMap::new(),
            chat_expanded_tool_lines: std::collections::HashSet::new(),
            chat_busy: false,
            chat_streaming: None,
            chat_tool_pending: None,
            chat_tool_running: None,
            chat_tool_running_detail: None,
            chat_reasoning: None,
            chat_activity_flow: None,
            chat_reasoning_compressing: false,
            chat_render_revision: 0,
            chat_history_revision: 0,
            chat_scroll_from_bottom: 0,
            chat_session_id: None,
            chat_pending_approval: None,
            approval_dialog: None,
            approval_decision_in_flight: None,
            last_pending_approval_count: 0,
            chat_context_visible: false,
            chat_context: None,
            chat_context_scroll_from_bottom: 0,
            chat_context_revision: 0,
            chat_pane_focus: ChatPaneFocus::Messages,
            detail_scroll_line: 0,
            detail_select: None,
            detail_selecting: false,
            dashboard_fold: DashboardFoldState::default(),
            digest_folded_sections: default_folded_digest_sections(),
            pr_overview_cache: HashMap::new(),
            pr_overview_fetching: None,
            attach_mode: false,
            dashboard_digest_pr_index: 0,
            dashboard_digest_pr_digest_idx: None,
        }
    }

    pub fn pr_overview_key(repo: &str, number: u32) -> String {
        format!("{repo}#{number}")
    }

    pub fn selected_pr_overview(&self) -> Option<&str> {
        let filtered = self.sorted_filtered_prs();
        let pr = filtered.get(self.selected_index)?;
        self.pr_overview_cache
            .get(&Self::pr_overview_key(&pr.repo, pr.number))
            .map(|s| s.as_str())
    }

    pub fn selected_pr_overview_loading(&self) -> bool {
        let filtered = self.sorted_filtered_prs();
        let Some(pr) = filtered.get(self.selected_index) else {
            return false;
        };
        let key = Self::pr_overview_key(&pr.repo, pr.number);
        self.pr_overview_fetching.as_ref() == Some(&key)
            && !self.pr_overview_cache.contains_key(&key)
    }

    pub fn toggle_dashboard_section_fold(&mut self, section: DashboardSection) {
        match section {
            DashboardSection::Digests => self.dashboard_fold.digests = !self.dashboard_fold.digests,
        }
    }

    pub fn toggle_digest_section_fold(&mut self, title: &str) {
        if self.digest_folded_sections.contains(title) {
            self.digest_folded_sections.remove(title);
        } else {
            self.digest_folded_sections.insert(title.to_string());
        }
    }

    /// Collapse empty Dashboard list sections after store hydration.
    pub fn sync_dashboard_fold(&mut self) {
        if self.digest_history.is_empty() {
            self.dashboard_fold.digests = true;
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

    pub fn push_chat_line(&mut self, line: impl Into<String>) {
        const MAX_CHAT_LINES: usize = 300;
        self.chat_lines.push(line.into());
        if self.chat_lines.len() > MAX_CHAT_LINES {
            let drain = self.chat_lines.len() - MAX_CHAT_LINES;
            self.chat_lines.drain(0..drain);
            self.reindex_chat_indices(drain);
        }
        self.bump_chat_render();
        self.bump_chat_history();
    }

    pub fn record_chat_tool_output(&mut self, line_index: usize, output: String) {
        if !output.is_empty() {
            self.chat_tool_outputs.insert(line_index, output);
            self.bump_chat_history();
        }
    }

    /// Record the raw (uncompressed) reasoning trace for a transcript line.
    /// Called when LLM reasoning compression was applied; the summary lives in
    /// `chat_tool_outputs` at the same index.
    pub fn record_chat_reasoning_original(&mut self, line_index: usize, original: String) {
        if !original.is_empty() {
            self.chat_reasoning_originals.insert(line_index, original);
            self.bump_chat_history();
        }
    }

    pub fn toggle_chat_tool_expand(&mut self, line_index: usize) -> bool {
        if !self.chat_tool_outputs.contains_key(&line_index) {
            return false;
        }
        if self.chat_expanded_tool_lines.contains(&line_index) {
            self.chat_expanded_tool_lines.remove(&line_index);
        } else {
            self.chat_expanded_tool_lines.insert(line_index);
        }
        self.bump_chat_render();
        true
    }

    pub fn toggle_last_chat_tool_expand(&mut self) -> bool {
        let Some(idx) = self
            .chat_lines
            .iter()
            .enumerate()
            .rev()
            .find(|(i, line)| {
                self.chat_tool_outputs.contains_key(i)
                    && (line.starts_with("  ✓ ") || line.starts_with("  ✗ "))
            })
            .map(|(i, _)| i)
        else {
            return false;
        };
        self.toggle_chat_tool_expand(idx)
    }

    fn reindex_chat_indices(&mut self, drained: usize) {
        self.reindex_chat_tool_outputs(drained);
        if let Some(pending) = self.chat_pending_approval.as_mut() {
            if pending.line_index >= drained {
                pending.line_index -= drained;
            } else {
                self.chat_pending_approval = None;
            }
        }
    }

    /// Update chat transcript after an approval is approved or denied.
    pub fn resolve_chat_approval(&mut self, id: uuid::Uuid, approved: bool, detail: &str) {
        if let Some(pending) = &self.chat_pending_approval {
            if pending.id == id && pending.line_index < self.chat_lines.len() {
                let (mark, verb) = if approved {
                    ("✓", "approved")
                } else if detail.to_ascii_lowercase().contains("failed") {
                    ("✗", "failed")
                } else {
                    ("✗", "denied")
                };
                self.chat_lines[pending.line_index] =
                    format!("  {mark} approval {verb}: {}", pending.tool_name);
                self.chat_pending_approval = None;
                self.bump_chat_render();
            }
        }
        let mark = if approved { "✓" } else { "✗" };
        self.push_chat_line(format!("  {mark} {detail}"));
    }

    pub fn set_chat_pending_approval(&mut self, pending: Option<ChatPendingApproval>) {
        self.chat_pending_approval = pending;
        self.bump_chat_render();
    }

    pub fn chat_approval_target_id(&self) -> Option<uuid::Uuid> {
        self.chat_pending_approval
            .as_ref()
            .map(|p| p.id)
            .or_else(|| self.approvals.first().map(|a| a.id))
    }

    pub fn can_decide_approval_inline(&self) -> bool {
        self.approval_dialog.is_some() || self.chat_approval_target_id().is_some()
    }

    pub fn open_approval_dialog(
        &mut self,
        id: uuid::Uuid,
        tool_name: String,
        description: String,
        tool_args_json: Option<String>,
    ) {
        if self.approval_decision_in_flight.is_some() {
            return;
        }
        self.approval_dialog = Some(ApprovalDialog {
            id,
            tool_name,
            description,
            tool_args_json,
            choice: ApprovalDialogChoice::Approve,
            deciding: false,
            opened_at: Instant::now(),
        });
    }

    pub fn open_approval_dialog_from(&mut self, approval: &crate::store::Approval) {
        let tool_name = approval_tool_name(approval);
        let tool_args_json =
            crate::approval_payload::resolve_approval_tool_args(None, Some(approval));
        self.open_approval_dialog(
            approval.id,
            tool_name,
            approval.description.clone(),
            tool_args_json,
        );
    }

    /// After store hydrate, open modal when workflow approvals grew (attach / refresh).
    pub fn maybe_notify_new_workflow_approvals(&mut self, prev_pending_count: usize) {
        self.last_pending_approval_count = self.approvals.len();
        if !self.config.chat.auto_approve_mutations
            && self.approval_dialog.is_none()
            && self.approvals.len() > prev_pending_count
        {
            if let Some(approval) = self.approvals.first().cloned() {
                self.open_approval_dialog_from(&approval);
            }
        }
    }

    pub fn close_approval_dialog(&mut self) {
        self.approval_dialog = None;
    }

    /// Lock before spawning approve/deny. Returns false if one is already in flight.
    pub fn try_begin_approval_decision(&mut self, id: uuid::Uuid, approve: bool) -> bool {
        if self.approval_decision_in_flight.is_some() {
            return false;
        }
        self.approval_decision_in_flight = Some(id);
        if let Some(dialog) = &mut self.approval_dialog {
            if dialog.id == id {
                dialog.deciding = true;
            }
        }
        self.status = if approve {
            "approval: approving…".into()
        } else {
            "approval: denying…".into()
        };
        true
    }

    pub fn finish_approval_decision(&mut self, id: uuid::Uuid) {
        if self.approval_decision_in_flight == Some(id) {
            self.approval_decision_in_flight = None;
        }
    }

    pub fn approval_decision_busy(&self) -> bool {
        self.approval_decision_in_flight.is_some()
    }

    pub fn toggle_approval_dialog_choice(&mut self) {
        if let Some(dialog) = &mut self.approval_dialog {
            if dialog.deciding {
                return;
            }
            dialog.choice = match dialog.choice {
                ApprovalDialogChoice::Approve => ApprovalDialogChoice::Deny,
                ApprovalDialogChoice::Deny => ApprovalDialogChoice::Approve,
            };
        }
    }

    /// Re-link a pending approval row after reload when the transcript still shows it.
    pub fn rehydrate_chat_pending_approval(&mut self) {
        if self.approvals.is_empty() {
            self.chat_pending_approval = None;
            return;
        }
        for (i, line) in self.chat_lines.iter().enumerate().rev().take(48) {
            for approval in &self.approvals {
                let id_str = approval.id.to_string();
                if !line.contains(&id_str) {
                    continue;
                }
                let tool_name = approval_tool_label(line, approval);
                self.chat_pending_approval = Some(ChatPendingApproval {
                    id: approval.id,
                    session_id: self.chat_session_id.unwrap_or_else(uuid::Uuid::nil),
                    tool_name,
                    tool_args_json: String::new(),
                    line_index: i,
                });
                return;
            }
        }
    }

    fn reindex_chat_tool_outputs(&mut self, drained: usize) {
        self.chat_tool_outputs = self
            .chat_tool_outputs
            .drain()
            .filter_map(|(idx, body)| {
                if idx >= drained {
                    Some((idx - drained, body))
                } else {
                    None
                }
            })
            .collect();
        self.chat_reasoning_originals = self
            .chat_reasoning_originals
            .drain()
            .filter_map(|(idx, body)| {
                if idx >= drained {
                    Some((idx - drained, body))
                } else {
                    None
                }
            })
            .collect();
        self.chat_expanded_tool_lines = self
            .chat_expanded_tool_lines
            .iter()
            .filter_map(|idx| idx.checked_sub(drained))
            .collect();
    }

    pub fn set_chat_streaming(&mut self, text: Option<String>) {
        self.chat_streaming = text;
        self.bump_chat_render();
    }

    pub fn set_chat_tool_pending(&mut self, label: Option<String>) {
        self.chat_tool_pending = label;
        self.bump_chat_render();
    }

    pub fn set_chat_tool_running(&mut self, name: Option<String>) {
        self.chat_tool_running = name;
        if self.chat_tool_running.is_none() {
            self.chat_tool_running_detail = None;
        }
        self.bump_chat_render();
    }

    pub fn set_chat_tool_running_detail(&mut self, detail: Option<String>) {
        self.chat_tool_running_detail = detail;
        self.bump_chat_render();
    }

    pub fn set_chat_reasoning(&mut self, text: Option<String>) {
        if text.is_some() {
            self.chat_reasoning_compressing = false;
        }
        self.chat_reasoning = text;
        self.bump_chat_render();
    }

    pub fn set_chat_activity_flow(&mut self, flow: Option<ChatActivityFlow>) {
        self.chat_activity_flow = flow;
        self.bump_chat_render();
    }

    pub fn set_chat_reasoning_compressing(&mut self, active: bool) {
        self.chat_reasoning_compressing = active;
        self.bump_chat_render();
    }

    pub fn set_chat_context(&mut self, snapshot: ContextSnapshot) {
        self.chat_context = Some(snapshot);
        self.chat_context_revision = self.chat_context_revision.wrapping_add(1);
    }

    pub fn toggle_chat_context_panel(&mut self) {
        self.chat_context_visible = !self.chat_context_visible;
        if !self.chat_context_visible {
            self.chat_context_scroll_from_bottom = 0;
            self.chat_pane_focus = ChatPaneFocus::Messages;
        }
        self.bump_chat_render();
    }

    pub fn chat_pane_focus_is_context(&self) -> bool {
        self.chat_context_visible && self.chat_pane_focus == ChatPaneFocus::Context
    }

    pub fn scroll_focused_chat_pane_line_up(&mut self) {
        if self.chat_pane_focus_is_context() {
            self.chat_context_scroll_from_bottom =
                self.chat_context_scroll_from_bottom.saturating_add(1);
        } else {
            self.chat_scroll_from_bottom = self.chat_scroll_from_bottom.saturating_add(1);
        }
    }

    pub fn scroll_focused_chat_pane_line_down(&mut self) {
        if self.chat_pane_focus_is_context() {
            self.chat_context_scroll_from_bottom =
                self.chat_context_scroll_from_bottom.saturating_sub(1);
        } else {
            self.chat_scroll_from_bottom = self.chat_scroll_from_bottom.saturating_sub(1);
        }
    }

    pub fn scroll_focused_chat_pane_page_up(&mut self) {
        const PAGE: u16 = 8;
        if self.chat_pane_focus_is_context() {
            self.chat_context_scroll_from_bottom =
                self.chat_context_scroll_from_bottom.saturating_add(PAGE);
        } else {
            self.chat_scroll_from_bottom = self.chat_scroll_from_bottom.saturating_add(PAGE);
        }
    }

    pub fn scroll_focused_chat_pane_page_down(&mut self) {
        const PAGE: u16 = 8;
        if self.chat_pane_focus_is_context() {
            self.chat_context_scroll_from_bottom =
                self.chat_context_scroll_from_bottom.saturating_sub(PAGE);
        } else {
            self.chat_scroll_from_bottom = self.chat_scroll_from_bottom.saturating_sub(PAGE);
        }
    }

    pub fn scroll_focused_chat_pane_to_latest(&mut self) {
        if self.chat_pane_focus_is_context() {
            self.chat_context_scroll_from_bottom = 0;
        } else {
            self.chat_scroll_from_bottom = 0;
        }
    }

    /// Chat turn phase for the status bar (thinking / tool / streaming / reasoning).
    pub fn chat_turn_phase(&self) -> Option<&'static str> {
        if !self.chat_busy {
            return None;
        }
        if self.chat_tool_pending.is_some() || self.chat_tool_running.is_some() {
            return Some("tool");
        }
        if self.chat_streaming.is_some() {
            return Some("streaming");
        }
        if self.chat_reasoning_compressing {
            return Some("summarizing");
        }
        if self.chat_reasoning.is_some() {
            return Some("reasoning");
        }
        if self.chat_activity_flow.is_some() {
            return Some("activity");
        }
        Some("model")
    }

    pub fn invalidate_render_cache(&mut self) {
        self.bump_chat_render();
    }

    fn bump_chat_render(&mut self) {
        self.chat_render_revision = self.chat_render_revision.wrapping_add(1);
    }

    fn bump_chat_history(&mut self) {
        self.chat_history_revision = self.chat_history_revision.wrapping_add(1);
    }

    pub fn selected_approval(&self) -> Option<&Approval> {
        self.approvals.get(self.selected_index)
    }

    pub fn filtered_prs(&self) -> Vec<&PrSnapshot> {
        use crate::agent::parse::{ci_is_failing, ci_is_passing, is_review_required};
        self.prs
            .iter()
            .filter(|p| match self.pr_filter {
                PrFilter::All => true,
                PrFilter::ReviewBlocked => {
                    p.triage_note.as_deref() == Some("review blocked")
                        || (is_review_required(&p.review_summary) && ci_is_passing(&p.ci_summary))
                }
                PrFilter::CiFailing => ci_is_failing(&p.ci_summary),
                PrFilter::MyPrs => p
                    .triage_note
                    .as_deref()
                    .is_some_and(|n| n.starts_with("my pr:")),
            })
            .collect()
    }

    pub fn sorted_filtered_prs(&self) -> Vec<&PrSnapshot> {
        let mut prs = self.filtered_prs();
        match self.pr_sort {
            PrSort::Updated => {
                prs.sort_by_key(|p| std::cmp::Reverse(p.fetched_at));
            }
            PrSort::CiStatus => {
                prs.sort_by_key(|p| ci_sort_key(&p.ci_summary));
            }
            PrSort::Number => {
                prs.sort_by_key(|p| std::cmp::Reverse(p.number));
            }
        }
        prs
    }

    /// Switch to PRs tab and select `repo#number` when present in store snapshots.
    pub fn jump_to_pr(&mut self, repo: &str, number: u32) -> bool {
        self.tab = Tab::Prs;
        self.pr_filter = PrFilter::All;
        let idx = self
            .sorted_filtered_prs()
            .iter()
            .position(|p| p.repo == repo && p.number == number);
        if let Some(i) = idx {
            self.selected_index = i;
            self.reset_detail_scroll();
            true
        } else {
            false
        }
    }

    pub fn filtered_logs(&self) -> Vec<&LogLine> {
        self.logs
            .iter()
            .filter(|l| self.log_filter.matches(&l.level))
            .collect()
    }

    pub fn push_chat_input_history(&mut self, msg: String) {
        if self.chat_input_history.last() != Some(&msg) {
            self.chat_input_history.push(msg);
            if self.chat_input_history.len() > 100 {
                let drain = self.chat_input_history.len() - 100;
                self.chat_input_history.drain(0..drain);
            }
        }
        self.chat_history_pos = None;
    }

    pub fn recall_chat_history_up(&mut self) {
        if self.chat_input_history.is_empty() {
            return;
        }
        let pos = self
            .chat_history_pos
            .unwrap_or(self.chat_input_history.len());
        if pos == 0 {
            return;
        }
        let new_pos = pos - 1;
        self.chat_input = self.chat_input_history[new_pos].clone();
        self.chat_history_pos = Some(new_pos);
    }

    pub fn recall_chat_history_down(&mut self) {
        let Some(pos) = self.chat_history_pos else {
            return;
        };
        if pos + 1 >= self.chat_input_history.len() {
            self.chat_history_pos = None;
            self.chat_input.clear();
        } else {
            let new_pos = pos + 1;
            self.chat_input = self.chat_input_history[new_pos].clone();
            self.chat_history_pos = Some(new_pos);
        }
    }

    pub fn clear_chat_transcript(&mut self) {
        self.chat_lines.clear();
        self.chat_tool_outputs.clear();
        self.chat_reasoning_originals.clear();
        self.chat_assistant_ids.clear();
        self.chat_expanded_tool_lines.clear();
        self.chat_pending_approval = None;
        self.set_chat_streaming(None);
        self.set_chat_tool_pending(None);
        self.set_chat_tool_running(None);
        self.set_chat_reasoning(None);
        self.set_chat_activity_flow(None);
        self.chat_reasoning_compressing = false;
        self.chat_scroll_from_bottom = 0;
        self.chat_context = None;
        self.chat_context_scroll_from_bottom = 0;
        self.chat_context_revision = self.chat_context_revision.wrapping_add(1);
        self.invalidate_render_cache();
        self.bump_chat_history();
    }

    /// Clear visible transcript, context panel, and drop the persisted session binding.
    pub fn reset_chat_session(&mut self) {
        self.clear_chat_transcript();
        self.chat_session_id = None;
    }

    pub fn reset_detail_scroll(&mut self) {
        self.detail_scroll_line = 0;
        self.detail_select = None;
        self.detail_selecting = false;
        self.dashboard_digest_pr_digest_idx = None;
    }
}

fn ci_sort_key(summary: &str) -> u8 {
    let lower = summary.to_ascii_lowercase();
    if lower.contains("fail") || lower.contains("red") {
        0
    } else if lower.contains("pending") || lower.contains("wait") {
        1
    } else if lower.contains("ok") || lower.contains("green") || lower.contains("pass") {
        2
    } else {
        3
    }
}

pub type SharedState = Arc<tokio::sync::RwLock<AppState>>;

pub fn event_channel() -> (broadcast::Sender<AppEvent>, broadcast::Receiver<AppEvent>) {
    broadcast::channel(256)
}

pub async fn hydrate_from_store(
    state: &SharedState,
    store: &dyn Store,
) -> crate::error::Result<()> {
    let digest = store.latest_digest().await?;
    let recent_digests = store.list_digests(20).await?;
    let digest_history: Vec<DigestMeta> = recent_digests.iter().map(|d| d.meta()).collect();
    let digest_bodies: HashMap<NaiveDate, String> = recent_digests
        .into_iter()
        .map(|d| (d.date, d.body_md))
        .collect();
    let prs = store.list_pr_snapshots(None).await?;
    let approvals = store.list_pending_approvals().await?;

    let mut s = state.write().await;
    s.latest_digest = digest;
    s.digest_history = digest_history;
    s.digest_bodies = digest_bodies;
    s.prs = prs;
    s.approvals = approvals;
    s.last_pending_approval_count = s.approvals.len();
    s.sync_dashboard_fold();
    Ok(())
}

fn chat_tool_args_short(msg: &ChatMessage) -> Option<String> {
    let json = msg.tool_calls_json.as_deref()?;
    let args: serde_json::Value = serde_json::from_str(json).ok()?;
    let short = crate::agent::chat_loop::format_tool_args_short(&args);
    if short.is_empty() {
        None
    } else {
        Some(short)
    }
}

fn chat_tool_start_display_line(msg: &ChatMessage) -> String {
    let name = msg.tool_name.as_deref().unwrap_or("tool");
    match chat_tool_args_short(msg) {
        Some(args) => format!("  → {name}({args})"),
        None => format!("  → {name}"),
    }
}

fn chat_tool_result_display_line(msg: &ChatMessage) -> String {
    let name = msg.tool_name.as_deref().unwrap_or("tool");
    if crate::agent::context::is_tool_approval_pending_transcript(&msg.content)
        || msg.content.contains("awaiting approval")
    {
        return format!("  → approval: {name}");
    }
    let ok = !crate::agent::context::tool_transcript_indicates_failure(&msg.content);
    let mark = if ok { "✓" } else { "✗" };
    match chat_tool_args_short(msg) {
        Some(args) => format!("  {mark} {name}({args})"),
        None => format!("  {mark} {name}"),
    }
}

/// Map a stored chat message to a TUI transcript line (`you>`, `assistant>`, tool rows).
pub fn chat_message_display_line(msg: &ChatMessage) -> String {
    match msg.role {
        ChatRole::User => format!("you> {}", msg.content),
        ChatRole::Assistant => format!("assistant> {}", msg.content),
        ChatRole::Tool => chat_tool_result_display_line(msg),
        ChatRole::Harness => {
            let preview = msg.content.lines().next().unwrap_or(&msg.content);
            let preview = if preview.chars().count() > 100 {
                format!("{}…", preview.chars().take(100).collect::<String>())
            } else {
                preview.to_string()
            };
            format!("  ⚠ harness: {preview}")
        }
        ChatRole::Reasoning => {
            let body = crate::agent::context::strip_reasoning_summary_marker(&msg.content);
            let preview = body
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or(body);
            let preview = if preview.chars().count() > 100 {
                format!("{}…", preview.chars().take(100).collect::<String>())
            } else {
                preview.to_string()
            };
            format!("  … reasoning: {preview}")
        }
    }
}

pub async fn load_chat_session_ui(
    state: &mut AppState,
    store: &dyn Store,
    session_id: uuid::Uuid,
) -> crate::error::Result<()> {
    let session = store.get_chat_session(&session_id).await?.ok_or_else(|| {
        crate::error::CoworkerError::Store(format!("unknown chat session {session_id}"))
    })?;
    let messages = store.list_active_branch_messages(&session, 300).await?;
    // Capture the prior context BEFORE clear_chat_transcript() nulls it, so we
    // can preserve its discovered tools & skills across the transcript reload.
    let prev_context = state.chat_context.clone();
    state.clear_chat_transcript();
    state.chat_session_id = Some(session_id);
    for msg in &messages {
        if msg.role == ChatRole::Reasoning {
            let body =
                crate::agent::context::strip_reasoning_summary_marker(&msg.content).to_string();
            let idx = state.chat_lines.len();
            state.push_chat_line(chat_message_display_line(msg));
            state.record_chat_tool_output(idx, body);
            if let Some(original) = &msg.reasoning_original {
                state.record_chat_reasoning_original(idx, original.clone());
            }
            continue;
        }
        if msg.role == ChatRole::Tool {
            if msg
                .tool_name
                .as_deref()
                .is_some_and(crate::agent::chat_loop::is_flow_activity_tool)
            {
                continue;
            }
            if !crate::agent::context::is_tool_approval_pending_transcript(&msg.content)
                && !msg.content.contains("awaiting approval")
            {
                state.push_chat_line(chat_tool_start_display_line(msg));
            }
            let idx = state.chat_lines.len();
            state.push_chat_line(chat_message_display_line(msg));
            state.record_chat_tool_output(idx, msg.content.clone());
        } else {
            let idx = state.chat_lines.len();
            state.push_chat_line(chat_message_display_line(msg));
            if msg.role == ChatRole::Assistant {
                state.chat_assistant_ids.insert(idx, msg.id);
            }
        }
    }
    state.chat_scroll_from_bottom = 0;
    state.rehydrate_chat_pending_approval();

    // Rebuild the context panel's message-derived fields (turn, tokens,
    // message_count, messages) from the freshly loaded messages. Tools &
    // skills are NOT rediscovered on session load (that needs the full chat
    // discovery pipeline, which only runs during a chat turn), so we preserve
    // them from the existing chat_context when present — otherwise the panel
    // would blank out TOOLS/SKILLS after every session switch *and* after every
    // chat turn (apply_chat_turn_result calls this to reload the transcript).
    //
    // On a cold switch (no prior context, e.g. picking a session from the
    // picker before chatting) we seed:
    //   - skill names from the session's persisted runtime_state.loaded_skills
    //   - tool definitions from the static ToolCatalog for the configured
    //     tool_mode (these are config-fixed, not session-specific, so they're
    //     a faithful preview; warmed business tools get added on the next turn)
    let llm_messages: Vec<_> = messages.iter().map(chat_message_to_llm).collect();
    let turn = llm_messages.len().max(1) as u32;
    let budget = TokenBudget::from_config(64_000);
    let reasoning_originals = crate::agent::context::reasoning_originals_from_history(&messages);

    let merged = if let Some(prev) = prev_context {
        // Keep the previously discovered tools & skills; refresh the rest.
        let fresh = build_context_snapshot(
            &llm_messages,
            turn,
            &budget,
            &[],
            &[],
            None,
            crate::agent::context::ContextPanelSources {
                store_messages: Some(&messages),
                skill_registry: None,
                reasoning_originals: Some(&reasoning_originals),
            },
        );
        ContextSnapshot {
            tools_body: prev.tools_body,
            tools_tokens: prev.tools_tokens,
            tool_names: prev.tool_names,
            skill_blocks: prev.skill_blocks,
            skills_tokens: prev.skills_tokens,
            runtime_context_revision: prev.runtime_context_revision,
            ..fresh
        }
    } else {
        // Cold switch: discover tools from the static catalog + load the
        // full skill specs (body/description/frontmatter) from the persisted
        // runtime_state.loaded_skills so the Skills section shows real
        // content + token counts, not just names with 0 tokens.
        let native_tools = ToolCatalog::new().native_tool_definitions(state.config.chat.tool_mode);
        let persisted_skills: Vec<String> = store
            .get_chat_session(&session_id)
            .await
            .ok()
            .flatten()
            .map(|s| s.runtime_state.loaded_skills)
            .unwrap_or_default();
        // Load each skill spec from disk; skip any that fail (e.g. removed
        // skill files) so a missing skill doesn't blank the whole panel.
        let loaded_skills: Vec<_> = persisted_skills
            .iter()
            .filter_map(|name| {
                crate::engine::skill::load_skill(crate::engine::skill::resolve_skill_ref(name)).ok()
            })
            .collect();
        build_context_snapshot(
            &llm_messages,
            turn,
            &budget,
            &native_tools,
            &loaded_skills,
            None,
            crate::agent::context::ContextPanelSources {
                store_messages: Some(&messages),
                skill_registry: None,
                reasoning_originals: Some(&reasoning_originals),
            },
        )
    };
    state.set_chat_context(merged);
    Ok(())
}

fn approval_tool_name_for_kind(kind: &crate::store::ApprovalKind) -> String {
    match kind {
        crate::store::ApprovalKind::RerunFlaky => "ci_rerun_workflow".into(),
        crate::store::ApprovalKind::Backport => "pr_create_backport".into(),
        crate::store::ApprovalKind::PostComment => "pr_post_comment".into(),
        crate::store::ApprovalKind::IssueAddLabel => "issue_add_label".into(),
        crate::store::ApprovalKind::WriteFile => "write_file".into(),
        crate::store::ApprovalKind::EditFile => "edit_file".into(),
        crate::store::ApprovalKind::BashRun => "bash_run".into(),
        crate::store::ApprovalKind::PythonRun => "python_run".into(),
        crate::store::ApprovalKind::McpTool => "mcp_tool".into(),
    }
}

fn approval_tool_name(approval: &crate::store::Approval) -> String {
    if approval.kind == crate::store::ApprovalKind::McpTool {
        if let Some(body) = &approval.comment_body {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
                if let Some(name) = v.get("tool_name").and_then(|x| x.as_str()) {
                    return name.to_string();
                }
            }
        }
    }
    approval_tool_name_for_kind(&approval.kind)
}

fn approval_tool_label(line: &str, approval: &crate::store::Approval) -> String {
    if let Some(rest) = line
        .split("approval pending:")
        .nth(1)
        .or_else(|| line.split("approval queued:").nth(1))
    {
        let name = rest.split('(').next().unwrap_or(rest).trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    approval_tool_name(approval)
}

pub fn export_chat_transcript_markdown(state: &AppState) -> String {
    let mut out = String::from("# Chat transcript\n\n");
    if let Some(id) = state.chat_session_id {
        out.push_str(&format!("Session: `{id}`\n\n"));
    }
    for line in &state.chat_lines {
        out.push_str(line);
        out.push('\n');
        if line.starts_with("assistant> ") {
            out.push('\n');
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::format_tool_context_message;
    use crate::store::{ChatMessage, ChatRole};
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn chat_message_display_line_pr_get_diff_success_with_failed_to_in_patch() {
        let body = "Diff for acme/widget#1 (40 bytes):\n\n\
diff --git a/x.go b/x.go\n\
+  return errors.New(\"failed to save\")";
        let content = format_tool_context_message(
            "pr_get_diff",
            &serde_json::json!({"repo": "acme/widget", "pr_number": 1}),
            true,
            body,
        );
        let msg = ChatMessage {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            role: ChatRole::Tool,
            content,
            ts: Utc::now(),
            tool_name: Some("pr_get_diff".into()),
            tool_calls_json: None,
            reasoning_original: None,
            parent_message_id: None,
            branch_index: None,
        };
        let line = chat_message_display_line(&msg);
        assert!(
            line.starts_with("  ✓ pr_get_diff"),
            "expected success mark, got: {line}"
        );
    }

    #[test]
    fn chat_message_display_line_tool_error_prefix() {
        let content = format_tool_context_message(
            "pr_get_diff",
            &serde_json::json!({"repo": "acme/widget", "pr_number": 1}),
            false,
            "failed to fetch PR diff",
        );
        let msg = ChatMessage {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            role: ChatRole::Tool,
            content,
            ts: Utc::now(),
            tool_name: Some("pr_get_diff".into()),
            tool_calls_json: None,
            reasoning_original: None,
            parent_message_id: None,
            branch_index: None,
        };
        assert!(chat_message_display_line(&msg).starts_with("  ✗ "));
    }

    #[test]
    fn chat_message_display_line_skill_load_includes_args() {
        use chrono::Utc;
        use uuid::Uuid;
        let msg = ChatMessage {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            role: ChatRole::Tool,
            content: "tool_result(skill_load):\nargs: {\"name\":\"pr-review\"}\n\n### pr-review"
                .into(),
            ts: Utc::now(),
            tool_name: Some("skill_load".into()),
            tool_calls_json: Some(r#"{"name":"pr-review"}"#.into()),
            reasoning_original: None,
            parent_message_id: None,
            branch_index: None,
        };
        assert_eq!(
            chat_message_display_line(&msg),
            "  ✓ skill_load(name=pr-review)"
        );
        assert_eq!(
            chat_tool_start_display_line(&msg),
            "  → skill_load(name=pr-review)"
        );
    }

    #[test]
    fn chat_message_display_line_reasoning_summary() {
        let msg = ChatMessage {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            role: ChatRole::Reasoning,
            content: "[agent reasoning summary]\n\nChecked CI on PR #42.".into(),
            ts: Utc::now(),
            tool_name: None,
            tool_calls_json: None,
            reasoning_original: None,
            parent_message_id: None,
            branch_index: None,
        };
        let line = chat_message_display_line(&msg);
        assert!(
            line.starts_with("  … reasoning: Checked CI on PR #42."),
            "got: {line}"
        );
    }

    /// Switching sessions must rebuild the context panel's message-derived
    /// fields from the loaded messages immediately — otherwise the panel keeps
    /// showing the previous session's context (or empty) until the next chat
    /// turn. Tools & skills discovered earlier are preserved across the reload
    /// (this path also runs after every chat turn to reload the transcript).
    #[tokio::test]
    async fn load_chat_session_ui_rebuilds_context_and_preserves_tools_skills() {
        use crate::agent::chat_loop::ContextSkillBlock;
        use crate::store::sqlite::SqliteStore;
        use std::sync::Arc;
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let store =
            Arc::new(SqliteStore::open(dir.path().join("test.db"), false).expect("open store"))
                as Arc<dyn crate::store::Store>;

        let session = store
            .create_chat_session(Some("preview-test"), None)
            .await
            .expect("create session");
        let sid = session.id;
        let user_msg = ChatMessage {
            id: Uuid::new_v4(),
            session_id: sid,
            role: ChatRole::User,
            content: "hello world".into(),
            ts: Utc::now(),
            tool_name: None,
            tool_calls_json: None,
            reasoning_original: None,
            parent_message_id: None,
            branch_index: None,
        };
        let ai_msg = ChatMessage {
            id: Uuid::new_v4(),
            session_id: sid,
            role: ChatRole::Assistant,
            content: "hi there".into(),
            ts: Utc::now(),
            tool_name: None,
            tool_calls_json: None,
            reasoning_original: None,
            parent_message_id: None,
            branch_index: None,
        };
        store
            .append_chat_message(&user_msg)
            .await
            .expect("append user");
        store
            .append_chat_message(&ai_msg)
            .await
            .expect("append assistant");

        let mut state = AppState::new(
            serde_yaml::from_str(
                r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
chat: { enabled: true }
storage: { backend: json, path: ./data }
repos: [acme/widget]
"#,
            )
            .expect("parse config"),
            "coworker.yaml".into(),
        );
        // Pre-seed a context with tools & skills (simulating what chat_loop
        // produced on a prior turn) to prove they survive the reload.
        state.set_chat_context(ContextSnapshot {
            tools_body: "bash_run / read_file".into(),
            tools_tokens: 800,
            tool_names: vec!["bash_run".into(), "read_file".into()],
            skill_blocks: vec![ContextSkillBlock {
                name: "github-ops-tone".into(),
                body: "be concise".into(),
                tokens: 60,
                description: "secretary tone".into(),
                always: true,
                skills: vec![],
                tools: vec![],
                argument_hint: String::new(),
                intent_phrases: vec![],
                intent_bonus_keywords: vec![],
            }],
            skills_tokens: 60,
            ..Default::default()
        });

        load_chat_session_ui(&mut state, store.as_ref(), sid)
            .await
            .expect("load session");

        let ctx = state
            .chat_context
            .as_ref()
            .expect("context rebuilt after session load");
        // Message-derived fields refreshed from the loaded messages.
        assert!(ctx.message_count >= 2, "expected >=2 messages in context");
        assert!(ctx.message_tokens > 0, "expected nonzero message tokens");
        assert_eq!(state.chat_session_id, Some(sid));
        // Tools & skills preserved from the prior context — NOT blanked.
        assert_eq!(ctx.tool_names, vec!["bash_run", "read_file"]);
        assert_eq!(ctx.tools_body, "bash_run / read_file");
        assert_eq!(ctx.skill_blocks.len(), 1);
        assert_eq!(ctx.skill_blocks[0].name, "github-ops-tone");
        assert!(ctx.skill_blocks[0].always);
    }

    /// Cold session switch (no prior context) seeds skill names from the
    /// session's persisted runtime_state.loaded_skills so the Skills section
    /// isn't empty before the first chat turn.
    #[tokio::test]
    async fn load_chat_session_ui_seeds_skills_from_persisted_runtime_state() {
        use crate::store::sqlite::SqliteStore;
        use std::sync::Arc;
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let store =
            Arc::new(SqliteStore::open(dir.path().join("test.db"), false).expect("open store"))
                as Arc<dyn crate::store::Store>;

        // Create a session and persist loaded_skills in its runtime_state.
        let mut session = store
            .create_chat_session(Some("seeded-skills"), None)
            .await
            .expect("create session");
        session.runtime_state.loaded_skills = vec!["github-ops-tone".into(), "ci-triage".into()];
        store
            .update_chat_session(&session)
            .await
            .expect("persist runtime_state");
        let sid = session.id;
        store
            .append_chat_message(&ChatMessage {
                id: Uuid::new_v4(),
                session_id: sid,
                role: ChatRole::User,
                content: "hi".into(),
                ts: Utc::now(),
                tool_name: None,
                tool_calls_json: None,
                reasoning_original: None,
                parent_message_id: None,
                branch_index: None,
            })
            .await
            .expect("append");

        let mut state = AppState::new(
            serde_yaml::from_str(
                r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
chat: { enabled: true }
storage: { backend: json, path: ./data }
repos: [acme/widget]
"#,
            )
            .expect("parse config"),
            "coworker.yaml".into(),
        );
        // No prior chat_context — cold switch path.
        assert!(state.chat_context.is_none());

        load_chat_session_ui(&mut state, store.as_ref(), sid)
            .await
            .expect("load session");

        let ctx = state
            .chat_context
            .as_ref()
            .expect("context built on cold switch");
        let skill_names: Vec<_> = ctx.skill_blocks.iter().map(|s| s.name.clone()).collect();
        assert_eq!(skill_names, vec!["github-ops-tone", "ci-triage"]);
        // Skills are loaded from disk with real bodies + token counts, not
        // just names with 0 tokens.
        assert!(
            ctx.skill_blocks.iter().all(|s| s.tokens > 0),
            "expected nonzero skill tokens, got: {:?}",
            ctx.skill_blocks
                .iter()
                .map(|s| (s.name.clone(), s.tokens))
                .collect::<Vec<_>>()
        );
        assert!(
            ctx.skill_blocks.iter().all(|s| !s.body.is_empty()),
            "expected non-empty skill bodies"
        );
        // github-ops-tone has always: true in its frontmatter.
        let tone = ctx
            .skill_blocks
            .iter()
            .find(|s| s.name == "github-ops-tone")
            .expect("github-ops-tone present");
        assert!(tone.always, "github-ops-tone should be always-on");
        assert!(!tone.description.is_empty(), "description should be loaded");
        // Tools are discovered from the static ToolCatalog for the configured
        // tool_mode (Auto → lazy-native subset), so TOOLS is NOT empty.
        assert!(
            !ctx.tool_names.is_empty(),
            "expected tool names from static catalog, got: {:?}",
            ctx.tool_names
        );
        assert!(ctx.tool_names.contains(&"bash_run".to_string()));
        assert!(!ctx.tools_body.is_empty(), "expected non-empty tools_body");
    }
}
