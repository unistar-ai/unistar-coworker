use crate::agent::context::truncate_chars;

pub fn truncate_tool_output(text: &str, max_chars: usize) -> String {
    truncate_chars(text, max_chars)
}
