//! Soft activity glyphs — Braille rotation driven by TUI session `Instant`.

use std::sync::OnceLock;
use std::time::Instant;

const FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

const FRAME_MS: u128 = 120;

static SESSION_START: OnceLock<Instant> = OnceLock::new();

/// Reset animation epoch when the TUI session starts (unifies spinner phase).
pub fn reset_session() {
    let _ = SESSION_START.set(Instant::now());
}

fn elapsed_ms() -> u128 {
    SESSION_START
        .get()
        .map(|t| t.elapsed().as_millis())
        .unwrap_or(0)
}

/// Index into [`FRAMES`] based on session elapsed time.
pub fn tick() -> u8 {
    ((elapsed_ms() / FRAME_MS) as u8) % FRAMES.len() as u8
}

pub fn frame_char() -> char {
    FRAMES[tick() as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_cycle_in_range() {
        reset_session();
        assert!(FRAMES.contains(&frame_char()));
        assert_eq!(FRAMES.len(), 10);
    }

    #[test]
    fn session_tick_advances_with_time() {
        reset_session();
        let a = tick();
        std::thread::sleep(std::time::Duration::from_millis(FRAME_MS as u64 + 20));
        let b = tick();
        assert_ne!(a, b);
    }
}
