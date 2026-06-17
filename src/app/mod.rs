use std::collections::HashMap;
use std::sync::Arc;

use chrono::NaiveDate;
use tokio::sync::broadcast;

use crate::agent::chat_loop::{ChatProgress, ContextSnapshot};
use crate::config::Config;
use crate::store::{
    Approval, AuditEntry, BackportQueueItem, ChatMessage, ChatRole, Digest, DigestMeta,
    FlakyTestRollup, IssueSnapshot, LogLine, MainAlert, PrSnapshot, Store,
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
    Flaky = 6,
    Release = 7,
    Issues = 8,
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
            Tab::Flaky,
        ]);
        if config
            .workflows
            .get("release-duty")
            .map(|w| w.enabled)
            .unwrap_or(false)
        {
            tabs.push(Tab::Release);
        }
        if config
            .workflows
            .get("issue-triage")
            .map(|w| w.enabled)
            .unwrap_or(false)
        {
            tabs.push(Tab::Issues);
        }
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
            Tab::Flaky => "6 Flaky",
            Tab::Release => "7 Release",
            Tab::Issues => "8 Issues",
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

#[derive(Debug, Clone)]
pub struct ApprovalDialog {
    pub id: uuid::Uuid,
    pub tool_name: String,
    pub description: String,
    pub choice: ApprovalDialogChoice,
    /// True while an approve/deny request is in flight (blocks duplicate submits).
    pub deciding: bool,
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
    pub flaky_tests: Vec<FlakyTestRollup>,
    pub main_alerts: Vec<MainAlert>,
    pub backport_queue: Vec<BackportQueueItem>,
    pub issues: Vec<IssueSnapshot>,
    pub pr_filter: PrFilter,
    pub pr_sort: PrSort,
    pub log_filter: LogFilter,
    pub logs: Vec<LogLine>,
    /// Digest markdown bodies keyed by date (for Dashboard detail).
    pub digest_bodies: HashMap<NaiveDate, String>,
    pub selected_index: usize,
    pub status: String,
    pub engine_busy: bool,
    pub mcp_ok: bool,
    pub llm_ok: bool,
    pub chat_input: String,
    pub chat_input_history: Vec<String>,
    /// Index into `chat_input_history` while browsing; `None` = drafting new input.
    pub chat_history_pos: Option<usize>,
    pub chat_lines: Vec<String>,
    /// Tool output keyed by index in `chat_lines` (for expand-on-o).
    pub chat_tool_outputs: std::collections::HashMap<usize, String>,
    pub chat_expanded_tool_lines: std::collections::HashSet<usize>,
    pub chat_busy: bool,
    /// Partial assistant reply while LLM is streaming (shown in the tail status area).
    pub chat_streaming: Option<String>,
    /// In-progress tool JSON while the model streams `action:tool`.
    pub chat_tool_pending: Option<String>,
    /// MCP tool in flight between ToolStart and ToolDone.
    pub chat_tool_running: Option<String>,
    /// Ollama thinking stream (internal reasoning — not shown as the final answer).
    pub chat_reasoning: Option<String>,
    /// True while the harness summarizes streamed thinking before the next JSON step.
    pub chat_reasoning_compressing: bool,
    /// Bumped when chat content changes; drives render cache invalidation.
    pub chat_render_revision: u64,
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
    pub flaky_since_days: u32,
    pub flaky_repo_filter: Option<String>,
    pub security_digest_md: Option<String>,
    /// Vertical scroll offset (wrapped lines) for the Detail pane on non-Chat tabs.
    pub detail_scroll_line: u16,
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
            flaky_tests: vec![],
            main_alerts: vec![],
            backport_queue: vec![],
            issues: vec![],
            pr_filter: PrFilter::All,
            pr_sort: PrSort::Updated,
            log_filter: LogFilter::All,
            logs: vec![],
            digest_bodies: HashMap::new(),
            selected_index: 0,
            status: "ready".into(),
            engine_busy: false,
            mcp_ok: false,
            llm_ok: false,
            chat_input: String::new(),
            chat_input_history: vec![],
            chat_history_pos: None,
            chat_lines: vec![],
            chat_tool_outputs: std::collections::HashMap::new(),
            chat_expanded_tool_lines: std::collections::HashSet::new(),
            chat_busy: false,
            chat_streaming: None,
            chat_tool_pending: None,
            chat_tool_running: None,
            chat_reasoning: None,
            chat_reasoning_compressing: false,
            chat_render_revision: 0,
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
            flaky_since_days: 30,
            flaky_repo_filter: None,
            security_digest_md: None,
            detail_scroll_line: 0,
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
    }

    pub fn record_chat_tool_output(&mut self, line_index: usize, output: String) {
        if !output.is_empty() {
            self.chat_tool_outputs.insert(line_index, output);
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

    pub fn open_approval_dialog(&mut self, id: uuid::Uuid, tool_name: String, description: String) {
        if self.approval_decision_in_flight.is_some() {
            return;
        }
        self.approval_dialog = Some(ApprovalDialog {
            id,
            tool_name,
            description,
            choice: ApprovalDialogChoice::Approve,
            deciding: false,
        });
    }

    pub fn open_approval_dialog_from(&mut self, approval: &crate::store::Approval) {
        let tool_name = approval_tool_name_for_kind(&approval.kind);
        self.open_approval_dialog(approval.id, tool_name, approval.description.clone());
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
        self.bump_chat_render();
    }

    pub fn set_chat_reasoning(&mut self, text: Option<String>) {
        if text.is_some() {
            self.chat_reasoning_compressing = false;
        }
        self.chat_reasoning = text;
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
        if self.chat_tool_pending.is_some() {
            return Some("tool-json");
        }
        if self.chat_tool_running.is_some() {
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
        Some("model")
    }

    pub fn invalidate_render_cache(&mut self) {
        self.bump_chat_render();
    }

    fn bump_chat_render(&mut self) {
        self.chat_render_revision = self.chat_render_revision.wrapping_add(1);
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
        self.chat_expanded_tool_lines.clear();
        self.chat_pending_approval = None;
        self.set_chat_streaming(None);
        self.set_chat_tool_pending(None);
        self.set_chat_tool_running(None);
        self.set_chat_reasoning(None);
        self.chat_reasoning_compressing = false;
        self.chat_scroll_from_bottom = 0;
        self.invalidate_render_cache();
    }

    pub fn reset_detail_scroll(&mut self) {
        self.detail_scroll_line = 0;
    }

    pub fn selected_backport(&self) -> Option<&BackportQueueItem> {
        self.backport_queue.get(self.selected_index)
    }

    pub fn selected_issue(&self) -> Option<&IssueSnapshot> {
        self.issues.get(self.selected_index)
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

    let filters = {
        let s = state.read().await;
        (s.flaky_since_days, s.flaky_repo_filter.clone())
    };
    let flaky = store
        .list_flaky_tests(crate::store::FlakyQuery {
            repo: filters.1,
            since_days: Some(filters.0),
            limit: 50,
        })
        .await?;
    let main_alerts = store
        .list_main_alerts(crate::store::MainAlertQuery {
            repo: None,
            unacknowledged_only: true,
            since_hours: Some(72),
            limit: 20,
        })
        .await?;
    let backport_queue = store.list_backport_queue(None).await?;
    let issues = store.list_issue_snapshots(None).await?;

    let security_digest_md = load_security_digest(store).await?;

    let mut s = state.write().await;
    s.latest_digest = digest;
    s.digest_history = digest_history;
    s.digest_bodies = digest_bodies;
    s.prs = prs;
    s.approvals = approvals;
    s.last_pending_approval_count = s.approvals.len();
    s.flaky_tests = flaky;
    s.main_alerts = main_alerts;
    s.backport_queue = backport_queue;
    s.issues = issues;
    s.security_digest_md = security_digest_md;
    Ok(())
}

async fn load_security_digest(store: &dyn Store) -> crate::error::Result<Option<String>> {
    Ok(store
        .get_digest_by_skill("security-digest")
        .await?
        .map(|d| d.body_md))
}

/// Map a stored chat message to a TUI transcript line (`you>`, `assistant>`, tool rows).
pub fn chat_message_display_line(msg: &ChatMessage) -> String {
    match msg.role {
        ChatRole::User => format!("you> {}", msg.content),
        ChatRole::Assistant => format!("assistant> {}", msg.content),
        ChatRole::Tool => {
            let name = msg.tool_name.as_deref().unwrap_or("tool");
            if crate::agent::context::is_tool_approval_pending_transcript(&msg.content)
                || msg.content.contains("awaiting approval")
            {
                format!("  → approval: {name}")
            } else if crate::agent::context::tool_transcript_indicates_failure(&msg.content) {
                format!("  ✗ {name}")
            } else {
                format!("  ✓ {name}")
            }
        }
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
    let messages = store.list_chat_messages(&session_id, 300).await?;
    state.clear_chat_transcript();
    state.chat_session_id = Some(session_id);
    for msg in messages {
        if msg.role == ChatRole::Reasoning {
            continue;
        }
        if msg.role == ChatRole::Tool {
            let idx = state.chat_lines.len();
            state.push_chat_line(chat_message_display_line(&msg));
            state.record_chat_tool_output(idx, msg.content.clone());
        } else {
            state.push_chat_line(chat_message_display_line(&msg));
        }
    }
    state.chat_scroll_from_bottom = 0;
    state.rehydrate_chat_pending_approval();
    Ok(())
}

fn approval_tool_name_for_kind(kind: &crate::store::ApprovalKind) -> String {
    match kind {
        crate::store::ApprovalKind::RerunFlaky => "ci_rerun_workflow".into(),
        crate::store::ApprovalKind::Backport => "pr_create_backport".into(),
        crate::store::ApprovalKind::PostComment => "pr_post_comment".into(),
    }
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
    approval_tool_name_for_kind(&approval.kind)
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
        };
        assert!(chat_message_display_line(&msg).starts_with("  ✗ "));
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
        };
        let line = chat_message_display_line(&msg);
        assert!(
            line.starts_with("  … reasoning: Checked CI on PR #42."),
            "got: {line}"
        );
    }
}
