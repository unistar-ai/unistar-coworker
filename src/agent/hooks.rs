//! Lightweight chat turn hooks (built-in only; v1 has no external plugin loading).

use crate::agent::budget::TokenBudget;
use crate::agent::chat_discovery::ChatDiscoveryState;
use crate::agent::context::CompactionStrategy;
use crate::error::Result;

/// Mutable per-turn state passed to hooks.
pub struct TurnContext {
    pub token_budget: TokenBudget,
    pub estimated_tokens: u32,
    pub last_tool: Option<String>,
    pub compaction: CompactionStrategy,
    pub pending_warm_tools: Vec<String>,
}

pub trait ChatTurnHook: Send + Sync {
    fn before_llm_turn(&self, ctx: &mut TurnContext) -> Result<()>;
    fn after_tool_result(&self, ctx: &mut TurnContext, tool: &str, output: &str) -> Result<()>;
}

fn chain_warm_tools(tool: &str) -> &'static [&'static str] {
    match tool {
        "pr_get_ci_snapshot" | "ci_analyze_pr_failures" => &["ci_get_failure_digest"],
        "pr_list_changed_files" => &["pr_diff_risk_scan"],
        "issue_list_open" => &["issue_get"],
        _ => &[],
    }
}

struct WarmToolChainHook;

impl ChatTurnHook for WarmToolChainHook {
    fn before_llm_turn(&self, _ctx: &mut TurnContext) -> Result<()> {
        Ok(())
    }

    fn after_tool_result(&self, ctx: &mut TurnContext, tool: &str, _output: &str) -> Result<()> {
        for name in chain_warm_tools(tool) {
            ctx.pending_warm_tools.push((*name).to_string());
        }
        ctx.last_tool = Some(tool.to_string());
        Ok(())
    }
}

/// No-op placeholder — compaction runs in `trim_llm_messages_with_llm` before hooks.
struct CompactionTriggerHook;

impl ChatTurnHook for CompactionTriggerHook {
    fn before_llm_turn(&self, ctx: &mut TurnContext) -> Result<()> {
        let over_budget = ctx.estimated_tokens > ctx.token_budget.input_budget();
        let _ = (over_budget, ctx.compaction, ctx.last_tool.as_deref());
        Ok(())
    }

    fn after_tool_result(&self, _ctx: &mut TurnContext, _tool: &str, _output: &str) -> Result<()> {
        Ok(())
    }
}

pub struct HookRunner {
    hooks: Vec<Box<dyn ChatTurnHook>>,
}

impl HookRunner {
    pub fn builtin() -> Self {
        Self {
            hooks: vec![
                Box::new(WarmToolChainHook),
                Box::new(CompactionTriggerHook),
            ],
        }
    }

    pub fn before_llm_turn(&self, ctx: &mut TurnContext) -> Result<()> {
        for hook in &self.hooks {
            hook.before_llm_turn(ctx)?;
        }
        Ok(())
    }

    pub fn after_tool_result(
        &self,
        ctx: &mut TurnContext,
        tool: &str,
        output: &str,
    ) -> Result<()> {
        for hook in &self.hooks {
            hook.after_tool_result(ctx, tool, output)?;
        }
        Ok(())
    }
}

/// Short-circuit `tool_list` when session already cached the catalog.
pub fn tool_list_cached_response(state: &ChatDiscoveryState) -> Option<String> {
    state.cached_tool_list()
}
