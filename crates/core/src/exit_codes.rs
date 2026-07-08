//! Stable CLI exit codes (Pi-style scriptability; see `design-plans/pi-agent-inspired-optimizations.md`).

/// Success (process exits 0 implicitly on `Ok(())`; exposed for scripts/docs).
pub const EXIT_OK: i32 = 0;

/// Unhandled / operational error.
pub const EXIT_GENERAL: i32 = 1;

/// Configuration or environment error (`doctor` fail, missing config).
pub const EXIT_CONFIG: i32 = 2;

/// Mutating tool blocked awaiting approval (headless `--once` without `--yes`).
pub const EXIT_APPROVAL: i32 = 3;

/// Wall-clock or LLM/network timeout (`--timeout`).
pub const EXIT_TIMEOUT: i32 = 4;

use crate::error::CoworkerError;

/// Map a `CoworkerError` to an exit code for `main`.
pub fn exit_code_for_error(err: &CoworkerError) -> i32 {
    match err {
        CoworkerError::Config(_) => EXIT_CONFIG,
        CoworkerError::Store(_) | CoworkerError::Sqlite(_) => EXIT_CONFIG,
        _ => EXIT_GENERAL,
    }
}
