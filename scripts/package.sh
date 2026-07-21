#!/usr/bin/env bash
set -euo pipefail

# Package unistar-coworker: build web-ui + release binary, assemble deploy tree.
# Same layout for local workdir and GitHub Release archives.
#
# Layout (Claude-style project agent dir):
#   <output>/unistar-coworker          # binary
#   <output>/.coworker/                # config, skills, data, docs
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
#   START_AGENT_DATA_BACKUP=path    temp backup while rebuilding (preserves .coworker/data/)
#   START_AGENT_PROFILE=release|dev default release
#   START_AGENT_SKIP_BUILD=1        skip cargo (still assembles tree; set by parent launcher)
#   PACKAGE_VERSION=…               with PACKAGE_TRIPLE → tar to dist/
#   PACKAGE_TRIPLE=…                target triple for release archive name

# ── Config ────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PARENT_DIR="$(cd "$REPO_ROOT/.." && pwd)"
COWORKER_DIR="$REPO_ROOT"
BUILD_HELPERS="$SCRIPT_DIR/build-helpers.sh"
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

AGENT_HOME="$WORKDIR/.coworker"

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

[ -f "$BUILD_HELPERS" ] || { echo "error: build helpers not found at $BUILD_HELPERS" >&2; exit 1; }
# shellcheck source=build-helpers.sh
source "$BUILD_HELPERS"

# Prefer `.coworker/coworker.yaml`, fall back to legacy flat path.
find_existing_config() {
  if [ -f "$AGENT_HOME/coworker.yaml" ]; then
    echo "$AGENT_HOME/coworker.yaml"
  elif [ -f "$WORKDIR/coworker.yaml" ]; then
    echo "$WORKDIR/coworker.yaml"
  fi
}

# Prefer `.coworker/data`, fall back to legacy flat `data/`.
find_existing_data_dir() {
  if [ -d "$AGENT_HOME/data" ]; then
    echo "$AGENT_HOME/data"
  elif [ -d "$WORKDIR/data" ]; then
    echo "$WORKDIR/data"
  fi
}

# Rewrite legacy `./data…` storage paths to `.coworker/data…` when migrating.
normalize_storage_path_in_config() {
  local cfg="$1"
  [ -f "$cfg" ] || return 0
  if grep -qE '^\s*path:\s*\./data' "$cfg" 2>/dev/null; then
    # BSD/GNU sed portable in-place via temp file
    local tmp
    tmp="$(mktemp)"
    sed -E 's|^([[:space:]]*path:[[:space:]]*)\./data|\1.coworker/data|' "$cfg" > "$tmp"
    mv "$tmp" "$cfg"
    skip "normalized storage.path → .coworker/data in preserved config"
  fi
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
  BINARY="$(binary_release_abs)"
  local -a cargo_args=(build --features embed-web-ui -p unistar-coworker)
  case "$START_AGENT_PROFILE" in
    release) cargo_args=(build --release --features embed-web-ui -p unistar-coworker) ;;
  esac

  if [ "$START_AGENT_SKIP_BUILD" = 1 ]; then
    [ -f "$BINARY" ] || { echo "error: START_AGENT_SKIP_BUILD=1 but $BINARY missing" >&2; exit 1; }
    skip "START_AGENT_SKIP_BUILD=1; using $BINARY"
    return 0
  fi

  if ! binary_needs_rebuild; then
    skip "cargo inputs unchanged; using $BINARY"
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
  log "assemble package tree at $WORKDIR (agent home: .coworker/)"

  local config_backup=""
  local existing_config
  existing_config="$(find_existing_config)"
  if [ -n "$existing_config" ]; then
    config_backup="$(mktemp)"
    cp "$existing_config" "$config_backup"
  fi

  # Preserve llm-profile sidecar next to the active config when present.
  local profile_backup=""
  local existing_profile=""
  if [ -f "$AGENT_HOME/coworker.llm-profile" ]; then
    existing_profile="$AGENT_HOME/coworker.llm-profile"
  elif [ -f "$WORKDIR/coworker.llm-profile" ]; then
    existing_profile="$WORKDIR/coworker.llm-profile"
  fi
  if [ -n "$existing_profile" ]; then
    profile_backup="$(mktemp)"
    cp "$existing_profile" "$profile_backup"
  fi

  local existing_data
  existing_data="$(find_existing_data_dir)"
  if [ -n "$existing_data" ]; then
    rm -rf "$DATA_BACKUP"
    mv "$existing_data" "$DATA_BACKUP"
  fi

  rm -rf "$WORKDIR"
  mkdir -p "$AGENT_HOME"

  cp "$BINARY" "$WORKDIR/unistar-coworker"
  cp -R "$COWORKER_DIR/skills" "$AGENT_HOME/skills"
  cp -R "$TEMPLATE" "$AGENT_HOME/template"
  if [ -n "$config_backup" ] && [ -f "$config_backup" ]; then
    cp "$config_backup" "$AGENT_HOME/coworker.yaml"
    rm -f "$config_backup"
    normalize_storage_path_in_config "$AGENT_HOME/coworker.yaml"
    skip "preserved existing coworker.yaml under .coworker/"
  else
    cp "$TEMPLATE/coworker.yaml" "$AGENT_HOME/coworker.yaml"
  fi
  if [ -n "$profile_backup" ] && [ -f "$profile_backup" ]; then
    cp "$profile_backup" "$AGENT_HOME/coworker.llm-profile"
    rm -f "$profile_backup"
    skip "preserved coworker.llm-profile under .coworker/"
  fi
  cp "$TEMPLATE/AGENTS.md" "$AGENT_HOME/AGENTS.md"
  cp "$REPO_ROOT/coworker.example.yaml" "$AGENT_HOME/"
  cp "$REPO_ROOT/coworker.minimal.yaml" "$AGENT_HOME/"
  cp "$REPO_ROOT/README.md" "$AGENT_HOME/"
  cp "$REPO_ROOT/QUICKSTART.md" "$AGENT_HOME/"
  cp "$REPO_ROOT/QUICKSTART_CN.md" "$AGENT_HOME/"

  if [ -d "$DATA_BACKUP" ]; then
    mv "$DATA_BACKUP" "$AGENT_HOME/data"
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
  if [ -d "$DATA_BACKUP" ] && [ -d "$WORKDIR" ] && [ ! -d "$AGENT_HOME/data" ]; then
    mkdir -p "$AGENT_HOME"
    mv "$DATA_BACKUP" "$AGENT_HOME/data" 2>/dev/null || true
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
