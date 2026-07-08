#!/usr/bin/env bash
# Install Husky commit-msg hook (commitlint). Safe to re-run.
set -euo pipefail
cd "$(dirname "$0")/.."
npm install

echo "git hooks ready: commit-msg runs commitlint (see docs/COMMITS.md)"
