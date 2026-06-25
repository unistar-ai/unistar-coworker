use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use ratatui::style::Style;
use ratatui::text::Line;

use crate::tui::markdown;
use crate::tui::theme::ThemePalette;

struct DetailRenderCache {
    key: u64,
    width: u16,
    lines: Vec<Line<'static>>,
}

static CACHE: Mutex<DetailRenderCache> = Mutex::new(DetailRenderCache {
    key: 0,
    width: 0,
    lines: Vec::new(),
});

pub fn detail_body_cache_key(body: &str, width: u16) -> u64 {
    let mut h = DefaultHasher::new();
    body.hash(&mut h);
    width.hash(&mut h);
    h.finish()
}

pub fn cached_detail_markdown_lines(
    th: ThemePalette,
    body: &str,
    width: usize,
    cache_key: u64,
) -> Vec<Line<'static>> {
    let w = width.min(u16::MAX as usize) as u16;
    let mut cache = CACHE.lock().expect("detail render cache");
    if cache.key != cache_key || cache.width != w {
        let base = Style::default().fg(th.text);
        cache.lines = markdown::markdown_to_lines_in_width(th, body, base, Some(width.max(1)));
        cache.key = cache_key;
        cache.width = w;
    }
    cache.lines.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_cache_reuses_lines_for_same_body() {
        let th = ThemePalette::dark();
        let body = "## Title\n\n- bullet one\n- bullet two\n";
        let key = detail_body_cache_key(body, 80);
        let a = cached_detail_markdown_lines(th, body, 80, key);
        let b = cached_detail_markdown_lines(th, body, 80, key);
        assert_eq!(a, b);
        let key2 = detail_body_cache_key("other body", 80);
        let c = cached_detail_markdown_lines(th, "other body", 80, key2);
        assert_ne!(a, c);
    }
}
