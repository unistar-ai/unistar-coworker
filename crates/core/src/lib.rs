//! Core harness: config, store, LLM, GitHub, MCP, agent, engine, app state.

pub mod agent;
pub mod app;
pub mod approval_payload;
pub mod config;
pub mod diagnostics;
pub mod engine;
pub mod error;
pub mod exit_codes;
pub mod github;
pub mod llm;
pub mod logging;
pub mod mcp;
pub mod output;
pub mod repo;
pub mod store;
pub mod terminal;
pub mod upgrade;
