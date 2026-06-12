pub mod client;
pub mod ollama;

pub use client::{append_log_chunk, next_prior_summary, ClassifyVerdict, LlmClient};
