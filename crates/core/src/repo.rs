//! Resolve repo-relative paths (`skills/`, `prompts/`, etc.) from the workspace checkout root.

use std::path::{Path, PathBuf};

/// Checkout root (`unistar-coworker/`), parent of `crates/`.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Resolve a repo-relative path: use cwd when present, else fall back to [`repo_root`].
pub fn resolve_repo_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        return path.to_path_buf();
    }
    if path.exists() {
        return path.to_path_buf();
    }
    let from_root = repo_root().join(path);
    if from_root.exists() {
        return from_root;
    }
    path.to_path_buf()
}
