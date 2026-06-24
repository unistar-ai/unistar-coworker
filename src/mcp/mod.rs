//! Optional third-party MCP server pool (stdio + HTTP Streamable HTTP).

mod cap;
mod cancel;
mod client;
mod http;
mod lazy_adapter;
mod pool;
mod registry;
mod rpc;
mod stdio;

#[allow(unused_imports)]
pub use cancel::{is_cancelled_error, McpCancel, CHAT_CANCELLED};
pub use lazy_adapter::{
    federated_tool_describe, federated_tool_list, federated_tool_search,
};
pub use pool::{spawn_mcp_pool, McpPool, McpServerStatus};
