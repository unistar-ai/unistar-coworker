#!/usr/bin/env bash
set -euo pipefail

# start-agent.sh — build the React UI + Rust binary, refresh the workdir from
# template (preserving runtime data), and launch the agent.
#
# Usage (from repo root or via ./scripts/start-agent.sh):
#   ./scripts/start-agent.sh [serve|tui|chat] [args...]
#     serve   Web UI (default) → http://127.0.0.1:8787
#     tui     Terminal TUI
#     chat    CLI chat REPL (args forwarded to `unistar-coworker chat`)
#   Other subcommands (daemon, run-once, …) are forwarded as-is.
#
# Env (optional):
#   START_AGENT_WORKDIR=path        runtime workdir (default: ../workdir next to repo)
#   START_AGENT_DATA_BACKUP=path    temp backup while rebuilding workdir
#   START_AGENT_PROFILE=release|dev default release; dev links faster for local iteration
#   START_AGENT_FORCE_BUILD=1       always run cargo (ignore up-to-date binary)
#   START_AGENT_SKIP_BUILD=1        never run cargo (use existing binary)
#   PORT=8787                         Web UI bind port

# ── Config ────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PARENT_DIR="$(cd "$REPO_ROOT/.." && pwd)"
COWORKER_DIR="$REPO_ROOT"
WORKDIR="${START_AGENT_WORKDIR:-$PARENT_DIR/workdir}"
TEMPLATE="$REPO_ROOT/packaging/workdir-template"
DATA_BACKUP="${START_AGENT_DATA_BACKUP:-$PARENT_DIR/.data-backup}"
PORT="${PORT:-8787}"
BINARY=""  # set by build_binary()

# Cargo profile for the agent binary (release = deploy default; dev = faster link).
START_AGENT_PROFILE="${START_AGENT_PROFILE:-release}"
START_AGENT_FORCE_BUILD="${START_AGENT_FORCE_BUILD:-0}"
START_AGENT_SKIP_BUILD="${START_AGENT_SKIP_BUILD:-0}"

# ── Guards ────────────────────────────────────────────────────────────────

# Refuse to wipe dangerous WORKDIR values (empty, root, home, repo root).
case "$WORKDIR" in
  ""|"$HOME"|"/"|"$REPO_ROOT") echo "refusing to wipe WORKDIR=$WORKDIR" >&2; exit 1 ;;
esac

command -v cargo >/dev/null 2>&1 || { echo "error: cargo not found in PATH" >&2; exit 1; }
[ -d "$TEMPLATE" ]              || { echo "error: template not found at $TEMPLATE" >&2; exit 1; }

# ── Helpers ───────────────────────────────────────────────────────────────

log()  { printf '==> %s\n' "$*"; }
skip() { printf '    %s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }

# Return 0 when any path under $@ is newer than $1 (binary).
any_newer_than() {
  local ref="$1"
  shift
  [ -n "$(find "$@" -type f -newer "$ref" -print -quit 2>/dev/null)" ]
}

binary_release_path() {
  case "$START_AGENT_PROFILE" in
    dev)     echo "target/debug/unistar-coworker" ;;
    release) echo "target/release/unistar-coworker" ;;
    *)
      echo "error: START_AGENT_PROFILE must be 'dev' or 'release' (got '$START_AGENT_PROFILE')" >&2
      exit 1
      ;;
  esac
}

# True when we must invoke cargo (missing binary or inputs newer than binary).
binary_needs_rebuild() {
  [ -f "$BINARY" ] || return 0
  any_newer_than "$BINARY" \
    crates Cargo.toml Cargo.lock .cargo/config.toml prompts vendor/chromiumoxide \
    && return 0
  if [ -d web-ui/dist ]; then
    any_newer_than "$BINARY" web-ui/dist && return 0
  fi
  return 1
}

# ── Steps ─────────────────────────────────────────────────────────────────

# Build the React Web UI so build.rs can embed web-ui/dist/ into the binary.
# Skips `npm install` / `npm run build:fast` when nothing changed — this keeps
# dist/ untouched and lets cargo skip recompiling the coworker crate entirely.
build_web_ui() {
  if ! command -v npm >/dev/null 2>&1; then
    warn "npm not found; skipping React UI build (Web UI will return 503)"
    return 0
  fi
  log "web-ui"

  # Install deps if node_modules is missing or package-lock is newer.
  local need_install=0
  if [ ! -d web-ui/node_modules ] \
    || [ ! -f web-ui/node_modules/.package-lock.json ] \
    || [ web-ui/package-lock.json -nt web-ui/node_modules/.package-lock.json ]; then
    need_install=1
  fi
  [ "$need_install" = 1 ] && ( cd web-ui && npm install )

  # Build only when sources are newer than dist/index.html (or dist is missing).
  if [ ! -f web-ui/dist/index.html ] \
    || [ -n "$(find web-ui/src web-ui/index.html web-ui/package.json \
               web-ui/vite.config.ts web-ui/tailwind.config.ts \
               -type f -newer web-ui/dist/index.html 2>/dev/null | head -1)" ]; then
    ( cd web-ui && npm run build:fast )
  else
    skip "web-ui unchanged; skipping vite build (dist/ preserved)"
  fi
}

build_binary() {
  BINARY="$COWORKER_DIR/$(binary_release_path)"
  local -a cargo_args=(build --features embed-web-ui -p unistar-coworker)
  case "$START_AGENT_PROFILE" in
    release) cargo_args=(build --release --features embed-web-ui -p unistar-coworker) ;;
  esac

  if [ "$START_AGENT_SKIP_BUILD" = 1 ]; then
    [ -f "$BINARY" ] || { echo "error: START_AGENT_SKIP_BUILD=1 but $BINARY missing" >&2; exit 1; }
    skip "START_AGENT_SKIP_BUILD=1; using $BINARY"
    return 0
  fi

  if [ "$START_AGENT_FORCE_BUILD" != 1 ] && ! binary_needs_rebuild; then
    skip "unistar-coworker up to date ($START_AGENT_PROFILE, embed-web-ui); skipping cargo"
    return 0
  fi

  log "cargo ${cargo_args[*]}"
  cargo "${cargo_args[@]}"
}

# Wipe and recreate WORKDIR from template, preserving runtime data/ and
# syncing the freshly built binary + skills.
rebuild_workdir() {
  log "rebuild workdir from template"

  # Move runtime data out before wiping (don't pollute the template).
  if [ -d "$WORKDIR/data" ]; then
    rm -rf "$DATA_BACKUP"
    mv "$WORKDIR/data" "$DATA_BACKUP"
  fi

  rm -rf "$WORKDIR"
  mkdir "$WORKDIR"
  cp -r "$TEMPLATE"/. "$WORKDIR"/

  # Restore runtime data.
  if [ -d "$DATA_BACKUP" ]; then
    rm -rf "$WORKDIR/data"
    mv "$DATA_BACKUP" "$WORKDIR/data"
  fi

  # Copy the freshly built binary (skip when already identical).
  if [ ! -f "$WORKDIR/unistar-coworker" ] || ! cmp -s "$BINARY" "$WORKDIR/unistar-coworker"; then
    cp "$BINARY" "$WORKDIR/unistar-coworker"
  else
    skip "workdir binary unchanged"
  fi

  # Sync technique skills when the repo copy is newer.
  if [ ! -d "$WORKDIR/skills" ] \
    || any_newer_than "$WORKDIR/skills" "$COWORKER_DIR/skills"; then
    rm -rf "$WORKDIR/skills"
    cp -R "$COWORKER_DIR/skills" "$WORKDIR/skills"
  else
    skip "workdir skills unchanged"
  fi
}

free_port() {
  local pids pid
  # Only kill listeners — clients (e.g. a browser tab) may also show up in lsof.
  pids="$(lsof -ti:"$PORT" -sTCP:LISTEN 2>/dev/null)" || return 0
  [ -z "$pids" ] && return 0
  log "killing stale process on port $PORT (pid $pids)"
  for pid in $pids; do
    kill "$pid" 2>/dev/null || true
  done
  sleep 1
  # Escalate if a listener is still bound.
  pids="$(lsof -ti:"$PORT" -sTCP:LISTEN 2>/dev/null)" || return 0
  [ -z "$pids" ] && return 0
  for pid in $pids; do
    kill -9 "$pid" 2>/dev/null || true
  done
  sleep 0.5
}

usage() {
  cat <<EOF
用法: $(basename "$0") [serve|tui|chat] [args...]

  serve   Web UI（默认）→ http://127.0.0.1:${PORT}
  tui     终端 TUI
  chat    CLI 聊天（参数传给 unistar-coworker chat）

示例:
  $(basename "$0")              # Web UI
  $(basename "$0") tui          # TUI
  $(basename "$0") chat         # 交互式 chat REPL
  $(basename "$0") chat --once "总结 open PRs"

其它子命令（daemon、run-once 等）原样转发。
EOF
}

# ── Cleanup trap ──────────────────────────────────────────────────────────

# If the script dies during rebuild_workdir (after data was moved out but
# before it was restored), move the backup back so data isn't orphaned.
cleanup() {
  if [ -d "$DATA_BACKUP" ] && [ -d "$WORKDIR" ] && [ ! -d "$WORKDIR/data" ]; then
    mv "$DATA_BACKUP" "$WORKDIR/data" 2>/dev/null || true
  fi
}
trap cleanup EXIT

# ── Main ──────────────────────────────────────────────────────────────────

# Clear screen + scrollback, home cursor.
printf '\033[2J\033[3J\033[H'

cd "$COWORKER_DIR"
build_web_ui
build_binary
rebuild_workdir

# Default mode: Web UI. Explicit: serve | tui | chat. Everything else passes through.
case "${1:-}" in
  -h|--help|help)
    usage
    exit 0
    ;;
esac

if [ $# -eq 0 ]; then
  set -- serve
fi

case "$1" in
  serve)
    free_port
    ;;
  tui|chat)
    ;;
  *)
    # daemon, run-once, triage-pr, …
    ;;
esac

log "./unistar-coworker $*"
cd "$WORKDIR"
trap - EXIT  # disarm — launching successfully
exec ./unistar-coworker "$@"
