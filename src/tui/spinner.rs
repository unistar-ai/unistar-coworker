//! Soft activity glyphs — Braille rotation instead of terminal SLOW_BLINK.

const FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

const FRAME_MS: u128 = 120;

/// Index into [`FRAMES`] based on wall clock (stable across redraws).
pub fn tick() -> u8 {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    ((ms / FRAME_MS) as u8) % FRAMES.len() as u8
}

pub fn frame_char() -> char {
    FRAMES[tick() as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_cycle_in_range() {
        assert!(FRAMES.contains(&frame_char()));
        assert_eq!(FRAMES.len(), 10);
    }
}
