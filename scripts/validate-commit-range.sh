#!/usr/bin/env bash
# Validate all commits in a git revision range (for CI).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
FROM=${1:?from rev required}
TO=${2:?to rev required}

if ! git -C "$ROOT" rev-parse --verify "$FROM^{commit}" >/dev/null 2>&1; then
  echo "error: unknown revision: $FROM" >&2
  exit 1
fi
if ! git -C "$ROOT" rev-parse --verify "$TO^{commit}" >/dev/null 2>&1; then
  echo "error: unknown revision: $TO" >&2
  exit 1
fi

count=0
while IFS= read -r sha; do
  [[ -z $sha ]] && continue
  subject=$(git -C "$ROOT" log -1 --format=%s "$sha")
  if ! "$ROOT/scripts/validate-commit-msg.sh" --subject "$subject"; then
    echo "error: invalid commit ${sha:0:7}: $subject" >&2
    exit 1
  fi
  ((count++)) || true
done < <(git -C "$ROOT" rev-list "${FROM}..${TO}")

echo "commit message check ok (${count} commit(s))"
