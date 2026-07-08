#!/usr/bin/env bash
set -euo pipefail

# Fail if Cargo.toml workspace version != README.md / README_CN.md crate version lines.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$REPO_ROOT"

cargo_version="$(awk '
  /^\[workspace\.package\]/ { in_ws = 1; next }
  in_ws && /^version = / {
    match($0, /"[^"]+"/)
    print substr($0, RSTART + 1, RLENGTH - 2)
    exit
  }
' Cargo.toml)"

if [[ -z "$cargo_version" ]]; then
  echo "error: could not read [workspace.package].version from Cargo.toml" >&2
  exit 1
fi

readme_version="$(grep -E '^Crate version: \*\*[0-9]+\.[0-9]+\.[0-9]+\*\*' README.md \
  | sed -E 's/.*\*\*([0-9]+\.[0-9]+\.[0-9]+)\*\*.*/\1/' \
  | head -1)"

readme_cn_version="$(grep -E '^(Crate version|版本)：\*\*[0-9]+\.[0-9]+\.[0-9]+\*\*' README_CN.md \
  | sed -E 's/.*\*\*([0-9]+\.[0-9]+\.[0-9]+)\*\*.*/\1/' \
  | head -1)"

fail=0

if [[ -z "$readme_version" ]]; then
  echo "error: could not find Crate version line in README.md" >&2
  fail=1
elif [[ "$readme_version" != "$cargo_version" ]]; then
  echo "error: README.md Crate version ($readme_version) != Cargo.toml workspace version ($cargo_version)" >&2
  fail=1
fi

if [[ -z "$readme_cn_version" ]]; then
  echo "error: could not find Crate version line in README_CN.md" >&2
  fail=1
elif [[ "$readme_cn_version" != "$cargo_version" ]]; then
  echo "error: README_CN.md Crate version ($readme_cn_version) != Cargo.toml workspace version ($cargo_version)" >&2
  fail=1
fi

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo "version check ok: $cargo_version (Cargo.toml, README.md, README_CN.md)"
