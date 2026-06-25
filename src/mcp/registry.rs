use std::collections::HashMap;

use serde_json::Value;

use crate::agent::tool_catalog;
use crate::config::{McpApprovalConfig, McpExposeConfig, McpMutatingPolicy, McpServerConfig};
use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone)]
pub struct McpToolDescriptor {
    pub remote_name: String,
    pub description: String,
    pub input_schema: Value,
    pub read_only_hint: bool,
    pub destructive_hint: bool,
}

#[derive(Debug, Clone)]
pub struct GlobalToolEntry {
    pub global_name: String,
    pub server_id: String,
    pub remote_name: String,
    pub mutating: bool,
}

#[derive(Debug, Default)]
pub struct GlobalToolRegistry {
    by_global: HashMap<String, GlobalToolEntry>,
}

impl GlobalToolRegistry {
    pub fn resolve(&self, global_name: &str) -> Option<&GlobalToolEntry> {
        self.by_global.get(global_name)
    }

    pub fn contains(&self, global_name: &str) -> bool {
        self.by_global.contains_key(global_name)
    }

    pub fn entries(&self) -> impl Iterator<Item = &GlobalToolEntry> {
        self.by_global.values()
    }

    pub fn register_server(
        &mut self,
        server: &McpServerConfig,
        tools: &[McpToolDescriptor],
    ) -> Result<()> {
        let prefix = server
            .expose
            .prefix
            .clone()
            .unwrap_or_else(|| format!("{}_", server.id));

        for tool in tools {
            if !tool_exposed(&server.expose, &tool.remote_name) {
                continue;
            }
            let global_name = format!("{prefix}{}", tool.remote_name);
            if tool_catalog::is_catalog_tool(&global_name)
                || tool_catalog::is_lazy_native_tool(&global_name)
            {
                return Err(CoworkerError::Other(anyhow::anyhow!(
                    "mcp server {:?} tool {:?} maps to {:?} which conflicts with built-in harness tools",
                    server.id,
                    tool.remote_name,
                    global_name
                )));
            }
            let mutating = tool_is_mutating(tool, &server.approval);
            if mutating && server.approval.mutating == McpMutatingPolicy::Deny {
                continue;
            }
            if self.by_global.contains_key(&global_name) {
                return Err(CoworkerError::Other(anyhow::anyhow!(
                    "duplicate federated tool name {global_name:?}"
                )));
            }
            self.by_global.insert(
                global_name.clone(),
                GlobalToolEntry {
                    global_name,
                    server_id: server.id.clone(),
                    remote_name: tool.remote_name.clone(),
                    mutating,
                },
            );
        }
        Ok(())
    }

    pub fn remove_server(&mut self, server_id: &str) {
        self.by_global
            .retain(|_, entry| entry.server_id != server_id);
    }
}

fn tool_exposed(expose: &McpExposeConfig, remote_name: &str) -> bool {
    if expose.denylist.iter().any(|d| d == remote_name) {
        return false;
    }
    if expose.allowlist.is_empty() {
        return true;
    }
    expose.allowlist.iter().any(|a| a == remote_name)
}

fn tool_is_mutating(tool: &McpToolDescriptor, approval: &McpApprovalConfig) -> bool {
    if approval.tools.iter().any(|t| t == &tool.remote_name) {
        return true;
    }
    if tool.read_only_hint {
        return false;
    }
    tool.destructive_hint || !tool.read_only_hint
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::McpTransport;

    #[test]
    fn prefix_maps_remote_to_global_name() {
        let server = McpServerConfig {
            id: "slack".into(),
            enabled: true,
            transport: McpTransport::Stdio,
            command: Some("echo".into()),
            args: vec![],
            env: HashMap::new(),
            url: None,
            headers: HashMap::new(),
            expose: McpExposeConfig {
                prefix: Some("slack_".into()),
                allowlist: vec!["post_message".into()],
                denylist: vec![],
            },
            approval: McpApprovalConfig {
                mutating: McpMutatingPolicy::Required,
                tools: vec![],
            },
            startup: None,
            timeout_secs: None,
            skills: vec![],
        };
        let tools = vec![McpToolDescriptor {
            remote_name: "post_message".into(),
            description: "post".into(),
            input_schema: serde_json::json!({}),
            read_only_hint: false,
            destructive_hint: true,
        }];
        let mut reg = GlobalToolRegistry::default();
        assert!(reg.register_server(&server, &tools).is_ok());
        assert!(reg.contains("slack_post_message"));
    }
}
