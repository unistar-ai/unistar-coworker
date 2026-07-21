# Shared build freshness helpers for start-agent.sh and package.sh.
# Source with: REPO_ROOT set to the unistar-coworker repo root.

any_newer_than() {
  local ref="$1"
  shift
  [ -n "$(find "$@" -type f -newer "$ref" -print -quit 2>/dev/null)" ]
}

binary_release_path() {
  case "${START_AGENT_PROFILE:-release}" in
    dev)     echo "target/debug/unistar-coworker" ;;
    release) echo "target/release/unistar-coworker" ;;
    *)
      echo "error: START_AGENT_PROFILE must be 'dev' or 'release' (got '${START_AGENT_PROFILE:-}')" >&2
      return 1
      ;;
  esac
}

binary_release_abs() {
  echo "$REPO_ROOT/$(binary_release_path)"
}

# True when cargo must run (missing binary or Rust / prompt / web-ui *source* inputs changed).
# Intentionally does NOT compare web-ui/dist/ mtimes alone — dist often ends up newer than
# the binary after packaging (npm finishes after cargo, or harmless touch) and that used to
# force full release rebuilds even with zero source changes.
binary_needs_rebuild() {
  local binary
  binary="$(binary_release_abs)" || return 0
  [ -f "$binary" ] || return 0
  any_newer_than "$binary" \
    "$REPO_ROOT/crates" \
    "$REPO_ROOT/Cargo.toml" \
    "$REPO_ROOT/Cargo.lock" \
    "$REPO_ROOT/.cargo/config.toml" \
    "$REPO_ROOT/prompts" \
    "$REPO_ROOT/vendor/chromiumoxide" \
    "$REPO_ROOT/web-ui/src" \
    "$REPO_ROOT/web-ui/index.html" \
    "$REPO_ROOT/web-ui/package.json" \
    "$REPO_ROOT/web-ui/package-lock.json" \
    "$REPO_ROOT/web-ui/vite.config.ts" \
    "$REPO_ROOT/web-ui/tailwind.config.ts"
}

build_web_ui_if_needed() {
  if ! command -v npm >/dev/null 2>&1; then
    printf 'warning: npm not found; skipping React UI build (Web UI will return 503)\n' >&2
    return 0
  fi

  local need_install=0
  if [ ! -d "$REPO_ROOT/web-ui/node_modules" ] \
    || [ ! -f "$REPO_ROOT/web-ui/node_modules/.package-lock.json" ] \
    || [ "$REPO_ROOT/web-ui/package-lock.json" -nt "$REPO_ROOT/web-ui/node_modules/.package-lock.json" ]; then
    need_install=1
  fi
  if [ "$need_install" = 1 ]; then
    printf '==> web-ui\n'
    ( cd "$REPO_ROOT/web-ui" && npm install )
  fi

  if [ ! -f "$REPO_ROOT/web-ui/dist/index.html" ] \
    || [ -n "$(find "$REPO_ROOT/web-ui/src" "$REPO_ROOT/web-ui/index.html" \
               "$REPO_ROOT/web-ui/package.json" "$REPO_ROOT/web-ui/vite.config.ts" \
               "$REPO_ROOT/web-ui/tailwind.config.ts" \
               -type f -newer "$REPO_ROOT/web-ui/dist/index.html" 2>/dev/null | head -1)" ]; then
    printf '==> web-ui\n'
    ( cd "$REPO_ROOT/web-ui" && npm run build:fast )
  fi
}
