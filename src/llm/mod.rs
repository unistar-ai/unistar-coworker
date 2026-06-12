pub mod client;
pub mod ollama;

pub use client::{
    append_log_chunk, format_classify_digest_lines, llm_reason_text, next_prior_summary,
    ClassifyVerdict, LlmClient,
};
