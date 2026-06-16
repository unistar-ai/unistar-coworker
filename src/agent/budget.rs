/// Token budget for 64K context windows.
#[derive(Debug, Clone)]
pub struct TokenBudget {
    pub context_limit: u32,
    pub system_reserved: u32,
    pub tools_reserved: u32,
    pub output_reserved: u32,
}

impl TokenBudget {
    pub fn from_config(context_limit: u32) -> Self {
        Self {
            context_limit,
            system_reserved: 4_096,
            tools_reserved: 2_048,
            output_reserved: 10_240,
        }
    }

    pub fn input_budget(&self) -> u32 {
        self.context_limit
            .saturating_sub(self.system_reserved)
            .saturating_sub(self.tools_reserved)
            .saturating_sub(self.output_reserved)
    }

    /// ~25% of input for prior session turns (user/assistant/tool summaries).
    pub fn history_budget(&self) -> u32 {
        self.input_budget() / 4
    }

    /// ~55% for the current user message + tool results this turn.
    #[allow(dead_code)]
    pub fn turn_budget(&self) -> u32 {
        self.input_budget() * 11 / 20
    }

    /// Hard cap for system prompt (skill + store snapshot).
    pub fn system_budget(&self) -> u32 {
        self.input_budget() / 5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_budget_64k() {
        let b = TokenBudget::from_config(64_000);
        assert_eq!(b.input_budget(), 47_616);
        assert_eq!(b.history_budget(), 11_904);
    }
}
