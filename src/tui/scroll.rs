use ratatui::widgets::ScrollbarState;

/// Build scrollbar state that matches [`Paragraph::scroll`] line offsets.
///
/// Ratatui's thumb math assumes `content_length` is the number of scroll
/// positions (not total wrapped lines). When `viewport_content_length` is set
/// to the viewport height while `content_length` is total lines, the thumb
/// stops short of the track bottom even when the paragraph is scrolled to the end.
pub fn paragraph_scrollbar_state(total: u16, visible: u16, scroll_y: u16) -> ScrollbarState {
    let max_scroll = total.saturating_sub(visible);
    let scroll_slots = usize::from(max_scroll.saturating_add(1).max(1));
    ScrollbarState::new(scroll_slots).position(scroll_y as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_no_scroll_and_bottom_pin() {
        paragraph_scrollbar_state(5, 10, 0);
        paragraph_scrollbar_state(47, 18, 29);
    }
}
