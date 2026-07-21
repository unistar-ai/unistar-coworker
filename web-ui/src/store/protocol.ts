// TypeScript mirrors of the Rust WS protocol structs.
// Authoritative source: src/web/snapshot.rs (WebSnapshot / WebLivePatch / WebChatPatch).
// Contract tests in src/web/snapshot.rs::tests lock the field sets.

import type { ChatMessagePart } from "../tabs/chat/messageParts";

export interface ActivityFlow {
  kind: string;
  text: string;
}

export interface ContextMessage {
  role: string;
  tokens: number;
  content: string;
  /** Raw (uncompressed) thinking trace for reasoning rows. */
  reasoning_original?: string;
}

export interface SkillBlock {
  name: string;
  tokens: number;
  body: string;
  /** Frontmatter `description` — shown in the skill preview modal. */
  description?: string;
  /** Frontmatter `always: true` — always-on skills are flagged. */
  always?: boolean;
  /** Frontmatter `skills:` refs (technique skills this skill pulls in). */
  skills?: string[];
  /** Frontmatter `tools:` refs (business/harness tools this skill declares). */
  tools?: string[];
  /** Frontmatter `argument-hint` — usage cue shown in the preview. */
  argument_hint?: string;
  /** Frontmatter `intent_phrases` — lazy-routing trigger phrases. */
  intent_phrases?: string[];
  /** Frontmatter `intent_bonus_keywords` — bonus scoring substrings. */
  intent_bonus_keywords?: string[];
}

export interface ChatContext {
  turn: number;
  message_tokens: number;
  tools_tokens: number;
  tools_body: string;
  tool_names: string[];
  skills_tokens: number;
  skill_blocks: SkillBlock[];
  input_budget: number;
  context_limit: number;
  message_count: number;
  messages: ContextMessage[];
  runtime_context_revision: number | null;
  context_trimmed_turns: number;
  context_summary_note: string | null;
}

export interface PendingApproval {
  id: string;
  session_id: string;
  tool_name: string;
  tool_args_json: string;
}

export interface PendingUserQuestion {
  id: string;
  session_id: string;
  question: string;
  options: string[];
  context: string | null;
  tool_call_id: string;
  tool_args_json: string;
}

export interface ApprovalDialog {
  id: string;
  tool_name: string;
  description: string;
  tool_args_json: string;
  choice: string;
  deciding: boolean;
  approve_armed: boolean;
  approve_arm_ms_remaining: number;
}

export interface ApprovalRow {
  id: string;
  kind: string;
  description: string;
  created_at: string;
  repo: string | null;
  pr_number: number | null;
  run_id: number | null;
  target_branch: string | null;
  status: string;
  comment_body: string | null;
  issue_number: number | null;
  label: string | null;
}

export interface LogEntry {
  level: string;
  message: string;
  ts: string;
}

export interface McpServerStatus {
  id: string;
  connected: boolean;
  tool_count: number;
  last_error: string | null;
  last_rpc_ms: number | null;
  prefix: string;
}

export interface LlmProfileOption {
  id: string;
  model: string;
  base_url: string;
}

export interface WebSnapshot {
  tab: string;
  tabs: string[];
  status: string;
  engine_busy: boolean;
  engine_task_label: string | null;
  chat_enabled: boolean;
  chat_busy: boolean;
  chat_session_id: string | null;
  chat_lines: string[];
  chat_tool_outputs: Record<string, string>;
  chat_reasoning_originals: Record<string, string>;
  /** ISO-8601 timestamps keyed by chat_lines index. */
  chat_line_times: Record<string, string>;
  chat_assistant_ids: Record<string, string>;
  chat_history_revision: number;
  /** In-flight turn process parts from backend (`null` when idle). */
  chat_turn_parts: ChatMessagePart[] | null;
  /** Completed-turn process parts keyed by `you>` line index. */
  chat_history_turn_parts: Record<string, ChatMessagePart[]>;
  /** Older messages exist in store or were dropped from the in-memory window. */
  chat_older_available: boolean;
  chat_lines_truncated?: boolean;
  chat_older_in_store?: boolean;
  chat_context_revision: number;
  chat_streaming: string | null;
  chat_reasoning: string | null;
  chat_tool_running: string | null;
  chat_tool_running_detail: string | null;
  chat_tool_pending: string | null;
  chat_turn_phase: string | null;
  chat_reasoning_compressing: boolean;
  chat_activity_flow: ActivityFlow | null;
  chat_context_visible: boolean;
  chat_context: ChatContext;
  chat_pending_approval: PendingApproval | null;
  chat_pending_user_question: PendingUserQuestion | null;
  approval_dialog: ApprovalDialog | null;
  approvals: ApprovalRow[];
  log_filter: string;
  logs: LogEntry[];
  config_path: string;
  llm_model: string;
  llm_profile: string | null;
  llm_profile_options: LlmProfileOption[];
  github_ok: boolean;
  llm_ok: boolean;
  github_latency_ms: number | null;
  llm_latency_ms: number | null;
  mcp_servers: McpServerStatus[];
  auto_approve_mutations: boolean;
  ui_theme: string;
  app_version: string;
  upgrade_available: boolean;
  latest_release: string | null;
  release_url: string | null;
}

export interface WebLivePatch {
  _type: "live";
  status: string;
  chat_busy: boolean;
  chat_streaming: string | null;
  chat_reasoning: string | null;
  chat_tool_running: string | null;
  chat_tool_running_detail: string | null;
  chat_tool_pending: string | null;
  chat_turn_phase: string | null;
  chat_reasoning_compressing: boolean;
  chat_activity_flow: ActivityFlow | null;
}

export interface WebChatPatch {
  _type: "chat";
  status: string;
  chat_busy: boolean;
  chat_session_id: string | null;
  chat_lines: string[];
  chat_tool_outputs: Record<string, string>;
  chat_reasoning_originals: Record<string, string>;
  /** ISO-8601 timestamps keyed by chat_lines index. */
  chat_line_times: Record<string, string>;
  chat_assistant_ids: Record<string, string>;
  chat_history_revision: number;
  /** In-flight turn process parts from backend (`null` when idle). */
  chat_turn_parts: ChatMessagePart[] | null;
  /** Completed-turn process parts keyed by `you>` line index. */
  chat_history_turn_parts: Record<string, ChatMessagePart[]>;
  /** Older messages exist in store or were dropped from the in-memory window. */
  chat_older_available: boolean;
  chat_lines_truncated?: boolean;
  chat_older_in_store?: boolean;
  chat_context_revision: number;
  chat_streaming: string | null;
  chat_reasoning: string | null;
  chat_tool_running: string | null;
  chat_tool_running_detail: string | null;
  chat_tool_pending: string | null;
  chat_turn_phase: string | null;
  chat_reasoning_compressing: boolean;
  chat_activity_flow: ActivityFlow | null;
  chat_context_visible: boolean;
  chat_context: ChatContext;
  chat_pending_approval: PendingApproval | null;
  chat_pending_user_question: PendingUserQuestion | null;
  approval_dialog: ApprovalDialog | null;
}

export type WsMessage = WebSnapshot | WebLivePatch | WebChatPatch;

export function isLivePatch(m: WsMessage): m is WebLivePatch {
  return (m as WebLivePatch)._type === "live";
}

export function isChatPatch(m: WsMessage): m is WebChatPatch {
  return (m as WebChatPatch)._type === "chat";
}

export function isSnapshot(m: WsMessage): m is WebSnapshot {
  return !isLivePatch(m) && !isChatPatch(m);
}
