#!/usr/bin/env bash
set -euo pipefail

# Package unistar-coworker for deploy: build web-ui + Rust binary, refresh workdir
# from template (preserving runtime data/). Does not launch the agent — use the
# parent ./start-agent.sh for that.
#
# Usage (from repo root):
#   ./scripts/package.sh
#
# Env (optional):
#   START_AGENT_WORKDIR=path        runtime workdir (default: ../workdir next to repo)
#   START_AGENT_DATA_BACKUP=path    temp backup while rebuilding workdir
#   START_AGENT_PROFILE=release|dev default release; dev links faster for local iteration
#   START_AGENT_SKIP_BUILD=1        skip cargo (still syncs workdir; set by parent launcher)

# ── Config ────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PARENT_DIR="$(cd "$REPO_ROOT/.." && pwd)"
COWORKER_DIR="$REPO_ROOT"
WORKDIR="${START_AGENT_WORKDIR:-$PARENT_DIR/workdir}"
TEMPLATE="$REPO_ROOT/packaging/workdir-template"
DATA_BACKUP="${START_AGENT_DATA_BACKUP:-$PARENT_DIR/.data-backup}"
BINARY=""  # set by build_binary()

START_AGENT_PROFILE="${START_AGENT_PROFILE:-release}"
START_AGENT_SKIP_BUILD="${START_AGENT_SKIP_BUILD:-0}"

# ── Guards ────────────────────────────────────────────────────────────────

case "$WORKDIR" in
  ""|"$HOME"|"/"|"$REPO_ROOT") echo "refusing to wipe WORKDIR=$WORKDIR" >&2; exit 1 ;;
esac

command -v cargo >/dev/null 2>&1 || { echo "error: cargo not found in PATH" >&2; exit 1; }
[ -d "$TEMPLATE" ]              || { echo "error: template not found at $TEMPLATE" >&2; exit 1; }

# ── Helpers ───────────────────────────────────────────────────────────────

log()  { printf '==> %s\n' "$*"; }
skip() { printf '    %s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }

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

# ── Steps ─────────────────────────────────────────────────────────────────

build_web_ui() {
  if ! command -v npm >/dev/null 2>&1; then
    warn "npm not found; skipping React UI build (Web UI will return 503)"
    return 0
  fi
  log "web-ui"

  local need_install=0
  if [ ! -d web-ui/node_modules ] \
    || [ ! -f web-ui/node_modules/.package-lock.json ] \
    || [ web-ui/package-lock.json -nt web-ui/node_modules/.package-lock.json ]; then
    need_install=1
  fi
  [ "$need_install" = 1 ] && ( cd web-ui && npm install )

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

  log "cargo ${cargo_args[*]}"
  cargo "${cargo_args[@]}"
}

rebuild_workdir() {
  log "rebuild workdir from template"

  if [ -d "$WORKDIR/data" ]; then
    rm -rf "$DATA_BACKUP"
    mv "$WORKDIR/data" "$DATA_BACKUP"
  fi

  rm -rf "$WORKDIR"
  mkdir "$WORKDIR"
  cp -r "$TEMPLATE"/. "$WORKDIR"/

  if [ -d "$DATA_BACKUP" ]; then
    rm -rf "$WORKDIR/data"
    mv "$DATA_BACKUP" "$WORKDIR/data"
  fi

  if [ ! -f "$WORKDIR/unistar-coworker" ] || ! cmp -s "$BINARY" "$WORKDIR/unistar-coworker"; then
    cp "$BINARY" "$WORKDIR/unistar-coworker"
  else
    skip "workdir binary unchanged"
  fi

  if [ ! -d "$WORKDIR/skills" ] \
    || any_newer_than "$WORKDIR/skills" "$COWORKER_DIR/skills"; then
    rm -rf "$WORKDIR/skills"
    cp -R "$COWORKER_DIR/skills" "$WORKDIR/skills"
  else
    skip "workdir skills unchanged"
  fi
}

cleanup() {
  if [ -d "$DATA_BACKUP" ] && [ -d "$WORKDIR" ] && [ ! -d "$WORKDIR/data" ]; then
    mv "$DATA_BACKUP" "$WORKDIR/data" 2>/dev/null || true
  fi
}
trap cleanup EXIT

# ── Main ──────────────────────────────────────────────────────────────────

cd "$COWORKER_DIR"
build_web_ui
build_binary
rebuild_workdir
trap - EXIT
