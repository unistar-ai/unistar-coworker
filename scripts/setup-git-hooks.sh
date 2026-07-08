#!/usr/bin/env bash
# Point git at repo-managed hooks (commit-msg → validate-commit-msg.sh). Safe to re-run.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)

chmod +x \
  "$ROOT/scripts/validate-commit-msg.sh" \
  "$ROOT/scripts/validate-commit-range.sh" \
  "$ROOT/scripts/hooks/commit-msg"

git -C "$ROOT" config core.hooksPath scripts/hooks

echo "git hooks ready: core.hooksPath=scripts/hooks (see docs/COMMITS.md)"
