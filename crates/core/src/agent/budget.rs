/// Token budget for 64K (and other) context windows.
#[derive(Debug, Clone)]
pub struct TokenBudget {
    pub context_limit: u32,
    pub system_reserved: u32,
    pub tools_reserved: u32,
    pub output_reserved: u32,
}

impl TokenBudget {
    pub fn from_config(context_limit: u32) -> Self {
        // Output reserve tracks model max generation, not a full 10K block.
        let output_reserved = context_limit.saturating_div(12).max(4_096);
        Self {
            context_limit,
            system_reserved: 4_096,
            tools_reserved: 2_048,
            output_reserved,
        }
    }

    pub fn input_budget(&self) -> u32 {
        self.context_limit
            .saturating_sub(self.system_reserved)
            .saturating_sub(self.tools_reserved)
            .saturating_sub(self.output_reserved)
    }

    /// ~40% of input for prior session turns (was 25% — too tight for 64K windows).
    pub fn history_budget(&self) -> u32 {
        self.input_budget() * 2 / 5
    }

    /// Remaining headroom for the active user message + tool results this turn.
    #[allow(dead_code)]
    pub fn turn_budget(&self) -> u32 {
        self.input_budget()
            .saturating_sub(self.history_budget())
            .saturating_sub(self.system_budget())
    }

    /// System prompt cap (skill + store snapshot) — 30% of input.
    pub fn system_budget(&self) -> u32 {
        self.input_budget() * 3 / 10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_budget_64k() {
        let b = TokenBudget::from_config(64_000);
        assert_eq!(b.output_reserved, 5_333);
        assert_eq!(b.input_budget(), 52_523);
        assert_eq!(b.history_budget(), 21_009);
        assert_eq!(b.system_budget(), 15_756);
    }
}
