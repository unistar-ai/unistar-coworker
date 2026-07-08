#!/usr/bin/env bash
# Validate a single commit subject line (Conventional Commits 1.0.0).
# See docs/COMMITS.md. No Node/Rust runtime required.
set -euo pipefail

readonly TYPES='build|chore|ci|deps|docs|feat|fix|perf|refactor|revert|style|test'
readonly SCOPES='core|cli|web|tui|web-ui|ci|docker|release|packaging|docs|skills|deps'
readonly HEADER_RE="^(${TYPES})(\\((${SCOPES})\\))?(!)?: [a-z].*$"
readonly MAX_HEADER_LEN=100

usage() {
  cat <<'EOF' >&2
usage:
  validate-commit-msg.sh <commit-msg-file>   # git commit-msg hook
  validate-commit-msg.sh --subject <line>    # check one subject
  validate-commit-msg.sh --self-test         # run built-in examples
EOF
}

should_skip() {
  local line=$1
  if printf '%s\n' "$line" | grep -Eq '^(Merge|Revert)'; then
    return 0
  fi
  return 1
}

validate_subject() {
  local line=$1

  if should_skip "$line"; then
    return 0
  fi

  if ((${#line} > MAX_HEADER_LEN)); then
    echo "error: commit header exceeds ${MAX_HEADER_LEN} characters" >&2
    echo "  $line" >&2
    return 1
  fi

  if [[ "$line" =~ \.$ ]]; then
    echo "error: commit subject must not end with a period" >&2
    echo "  $line" >&2
    return 1
  fi

  if ! printf '%s\n' "$line" | grep -Eq "$HEADER_RE"; then
    echo "error: commit message must follow Conventional Commits (see docs/COMMITS.md)" >&2
    echo "  expected: <type>[scope]: <description>" >&2
    echo "  got:      $line" >&2
    return 1
  fi

  return 0
}

read_subject_from_file() {
  local file=$1
  sed -n '1p' "$file" | tr -d '\r'
}

run_self_test() {
  local ok=0 fail=0
  check() {
    local expect=$1
    local line=$2
    if validate_subject "$line"; then
      if [[ "$expect" == pass ]]; then
        ((ok++)) || true
      else
        echo "self-test: expected fail, got pass: $line" >&2
        ((fail++)) || true
      fi
    else
      if [[ "$expect" == fail ]]; then
        ((ok++)) || true
      else
        echo "self-test: expected pass, got fail: $line" >&2
        ((fail++)) || true
      fi
    fi
  }

  check pass "feat(cli): add doctor bundle export"
  check pass "fix(web-ui): handle react 19 types"
  check pass "ci: enforce commit messages"
  check pass "chore(deps): bump anyhow"
  check pass "feat(core)!: break config format"
  check pass "Merge pull request #1 from org/branch"
  check pass "Revert \"feat: bad idea\""
  check fail "bad message"
  check fail "feat: Ends with period."
  check fail "feat(BAD): wrong scope"
  check fail "Feat(cli): uppercase type"

  if ((fail > 0)); then
    echo "self-test: ${fail} failure(s)" >&2
    return 1
  fi
  echo "self-test: ${ok} checks passed"
  return 0
}

main() {
  case ${1:-} in
    --self-test)
      run_self_test
      ;;
    --subject)
      [[ $# -ge 2 ]] || {
        usage
        exit 2
      }
      validate_subject "$2"
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    "")
      usage
      exit 2
      ;;
    *)
      if [[ -f $1 ]]; then
        validate_subject "$(read_subject_from_file "$1")"
      else
        echo "error: not a file: $1" >&2
        exit 2
      fi
      ;;
  esac
}

main "$@"
