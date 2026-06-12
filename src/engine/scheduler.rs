//! Cron scheduler — Phase 2. v0.1 uses manual `r` / `run-once` only.

use crate::config::Config;

pub struct Scheduler {
    _config: Config,
}

impl Scheduler {
    pub fn new(config: Config) -> Self {
        Self { _config: config }
    }
}
