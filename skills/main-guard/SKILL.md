---
name: main-guard
description: Watch default-branch CI; alert immediately when main goes red.
---

# Main branch guard

1. For each repo, check recent workflow runs on the default branch.
2. If consecutive failures exceed threshold, write a `main_alert` and surface on Dashboard.
3. No LLM required for pure red/green — rules first, LLM for log summary if needed.
