---
name: review-radar
description: List PRs that are CI-green but blocked on review.
---

# Review blocker radar

1. `pr_list_open` per repo.
2. Filter: CI passing, `reviewDecision=REVIEW_REQUIRED`, not draft.
3. Append a "waiting for review" section to the digest or a dedicated snapshot.
