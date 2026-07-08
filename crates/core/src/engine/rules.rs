use crate::config::{RuleAction, RuleConfig};
use crate::llm::ClassifyVerdict;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleMatch {
    SuggestRerun,
    MarkFlaky,
    SkipLlm,
}

pub fn apply_rules(rules: &[RuleConfig], workflow: &str, error_text: &str) -> Option<RuleMatch> {
    let err_lower = error_text.to_ascii_lowercase();
    for rule in rules {
        if let Some(wf) = &rule.workflow {
            if wf != workflow {
                continue;
            }
        }
        if let Some(pat) = &rule.error_contains {
            if !err_lower.contains(&pat.to_ascii_lowercase()) {
                continue;
            }
        }
        return Some(match rule.then {
            RuleAction::SuggestRerun => RuleMatch::SuggestRerun,
            RuleAction::MarkFlaky => RuleMatch::MarkFlaky,
            RuleAction::SkipLlm => RuleMatch::SkipLlm,
        });
    }
    None
}

pub fn verdict_from_rule(m: RuleMatch) -> ClassifyVerdict {
    match m {
        RuleMatch::MarkFlaky | RuleMatch::SuggestRerun => ClassifyVerdict::Flaky,
        RuleMatch::SkipLlm => ClassifyVerdict::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuleConfig;

    #[test]
    fn matches_timeout_rule() {
        let rules = vec![RuleConfig {
            workflow: Some("test-integration".into()),
            error_contains: Some("timeout".into()),
            then: RuleAction::MarkFlaky,
        }];
        let m = apply_rules(&rules, "test-integration", "Error: connection timeout").unwrap();
        assert_eq!(m, RuleMatch::MarkFlaky);
    }
}
