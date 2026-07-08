#!/usr/bin/env bash
set -euo pipefail

# Package unistar-coworker: build web-ui + release binary, assemble deploy tree.
# Same layout for local workdir and GitHub Release archives.
#
# Usage (from repo root):
#   ./scripts/package.sh
#
# Local deploy (default output: ../workdir next to repo):
#   ./scripts/package.sh
#   START_AGENT_WORKDIR=./workdir ./scripts/package.sh
#
# GitHub Release (also writes dist/*.tar.gz + .sha256):
#   PACKAGE_VERSION=2.0.0 PACKAGE_TRIPLE=x86_64-unknown-linux-gnu ./scripts/package.sh
#
# Env (optional):
#   START_AGENT_WORKDIR=path        output tree (default: ../workdir, or dist/… when versioning)
#   START_AGENT_DATA_BACKUP=path    temp backup while rebuilding (preserves data/)
#   START_AGENT_PROFILE=release|dev default release
#   START_AGENT_SKIP_BUILD=1        skip cargo (still assembles tree; set by parent launcher)
#   PACKAGE_VERSION=…               with PACKAGE_TRIPLE → tar to dist/
#   PACKAGE_TRIPLE=…                target triple for release archive name

# ── Config ────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PARENT_DIR="$(cd "$REPO_ROOT/.." && pwd)"
COWORKER_DIR="$REPO_ROOT"
TEMPLATE="$REPO_ROOT/packaging/workdir-template"
DATA_BACKUP="${START_AGENT_DATA_BACKUP:-$PARENT_DIR/.data-backup}"
BINARY=""  # set by build_binary()

START_AGENT_PROFILE="${START_AGENT_PROFILE:-release}"
START_AGENT_SKIP_BUILD="${START_AGENT_SKIP_BUILD:-0}"
PACKAGE_VERSION="${PACKAGE_VERSION:-}"
PACKAGE_TRIPLE="${PACKAGE_TRIPLE:-}"
PACKAGE_VERSION="${PACKAGE_VERSION#v}"

if [ -n "$PACKAGE_VERSION" ] && [ -n "$PACKAGE_TRIPLE" ]; then
  PACKAGE_BASENAME="unistar-coworker-${PACKAGE_VERSION}-${PACKAGE_TRIPLE}"
  WORKDIR="${START_AGENT_WORKDIR:-$REPO_ROOT/dist/$PACKAGE_BASENAME}"
else
  WORKDIR="${START_AGENT_WORKDIR:-$PARENT_DIR/workdir}"
fi

# ── Guards ────────────────────────────────────────────────────────────────

case "$WORKDIR" in
  ""|"$HOME"|"/"|"$REPO_ROOT") echo "refusing to wipe WORKDIR=$WORKDIR" >&2; exit 1 ;;
esac

command -v cargo >/dev/null 2>&1 || { echo "error: cargo not found in PATH" >&2; exit 1; }
[ -d "$TEMPLATE" ]              || { echo "error: template not found at $TEMPLATE" >&2; exit 1; }
[ -f "$REPO_ROOT/coworker.example.yaml" ] \
  || { echo "error: coworker.example.yaml not found" >&2; exit 1; }

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
  if [ "$START_AGENT_PROFILE" = release ]; then
    CARGO_INCREMENTAL=0 cargo "${cargo_args[@]}"
  else
    cargo "${cargo_args[@]}"
  fi
}

assemble_tree() {
  log "assemble package tree at $WORKDIR"

  if [ -d "$WORKDIR/data" ]; then
    rm -rf "$DATA_BACKUP"
    mv "$WORKDIR/data" "$DATA_BACKUP"
  fi

  rm -rf "$WORKDIR"
  mkdir -p "$WORKDIR"

  cp "$BINARY" "$WORKDIR/unistar-coworker"
  cp -R "$COWORKER_DIR/skills" "$WORKDIR/skills"
  cp -R "$TEMPLATE" "$WORKDIR/template"
  cp "$TEMPLATE/coworker.yaml" "$WORKDIR/coworker.yaml"
  cp "$TEMPLATE/AGENTS.md" "$WORKDIR/AGENTS.md"
  cp "$REPO_ROOT/coworker.example.yaml" "$WORKDIR/"
  cp "$REPO_ROOT/coworker.minimal.yaml" "$WORKDIR/"
  cp "$REPO_ROOT/README.md" "$WORKDIR/"
  cp "$REPO_ROOT/QUICKSTART.md" "$WORKDIR/"
  cp "$REPO_ROOT/QUICKSTART_CN.md" "$WORKDIR/"

  if [ -d "$DATA_BACKUP" ]; then
    mv "$DATA_BACKUP" "$WORKDIR/data"
  fi
}

write_release_archive() {
  [ -n "$PACKAGE_VERSION" ] && [ -n "$PACKAGE_TRIPLE" ] || return 0

  log "release archive dist/${PACKAGE_BASENAME}.tar.gz"
  mkdir -p "$REPO_ROOT/dist"
  tar -czf "$REPO_ROOT/dist/${PACKAGE_BASENAME}.tar.gz" -C "$REPO_ROOT/dist" "$PACKAGE_BASENAME"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$REPO_ROOT/dist/${PACKAGE_BASENAME}.tar.gz" \
      > "$REPO_ROOT/dist/${PACKAGE_BASENAME}.tar.gz.sha256"
  else
    shasum -a 256 "$REPO_ROOT/dist/${PACKAGE_BASENAME}.tar.gz" \
      > "$REPO_ROOT/dist/${PACKAGE_BASENAME}.tar.gz.sha256"
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
assemble_tree
write_release_archive
trap - EXIT
