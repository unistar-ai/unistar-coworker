use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;
use tokio::sync::RwLock;

use crate::config::{McpConfig, McpMutatingPolicy, McpServerConfig, McpStartup};
use crate::engine::workflows::check_workflow_mcp_allowed;
use crate::error::{CoworkerError, Result};
use crate::mcp::cancel::{is_cancelled_error, McpCancel};
use crate::mcp::cap::truncate_tool_output;
use crate::mcp::client::McpClient;
use crate::mcp::registry::{GlobalToolEntry, GlobalToolRegistry, McpToolDescriptor};

#[derive(Debug, Clone)]
pub struct McpServerStatus {
    pub id: String,
    pub connected: bool,
    pub tool_count: u32,
    pub last_error: Option<String>,
    pub last_rpc_ms: Option<u128>,
    /// Expose prefix for UI source labels (`slack_` → tool `slack_post_message`).
    pub prefix: String,
}

struct ConnectedServer {
    client: McpClient,
    tools: Vec<McpToolDescriptor>,
}

pub struct McpPool {
    defaults: std::sync::RwLock<crate::config::McpDefaults>,
    servers: std::sync::RwLock<Vec<McpServerConfig>>,
    sessions: RwLock<HashMap<String, ConnectedServer>>,
    registry: RwLock<GlobalToolRegistry>,
    status: RwLock<HashMap<String, McpServerStatus>>,
}

impl McpPool {
    pub fn new(config: McpConfig) -> Self {
        let status = initial_status_map(&config.servers);
        Self {
            defaults: std::sync::RwLock::new(config.defaults),
            servers: std::sync::RwLock::new(config.servers),
            sessions: RwLock::new(HashMap::new()),
            registry: RwLock::new(GlobalToolRegistry::default()),
            status: RwLock::new(status),
        }
    }

    /// Read-only access to the servers config. Lock poisoning is treated as
    /// "no servers" rather than panicking the process — a poisoned lock means
    /// a writer panicked mid-update, and crashing the whole engine on a
    /// config-read path is disproportionate. The error is logged once.
    fn read_servers(&self) -> std::sync::RwLockReadGuard<'_, Vec<McpServerConfig>> {
        match self.servers.read() {
            Ok(g) => g,
            Err(e) => {
                tracing::error!("mcp servers lock poisoned: {e}; returning empty");
                // Poisoned guard still gives access via unwrap_or_else fallback
                // but we cannot fabricate a guard; fall back to clearing via a
                // panic-safe path is impossible without unsafe. Re-acquire with
                // the inner poison ignored to keep the process alive.
                self.servers.read().unwrap_or_else(|p| p.into_inner())
            }
        }
    }

    /// Read-only access to the mcp defaults. Same poison policy as
    /// [`read_servers`]: fall back to default rather than panic.
    fn read_defaults(&self) -> std::sync::RwLockReadGuard<'_, crate::config::McpDefaults> {
        match self.defaults.read() {
            Ok(g) => g,
            Err(e) => {
                tracing::error!("mcp defaults lock poisoned: {e}; returning defaults");
                self.defaults.read().unwrap_or_else(|p| p.into_inner())
            }
        }
    }

    /// Write access to the servers config. Poison is recovered via
    /// `into_inner()` so a previous failed writer does not block future updates.
    fn write_servers(&self) -> std::sync::RwLockWriteGuard<'_, Vec<McpServerConfig>> {
        match self.servers.write() {
            Ok(g) => g,
            Err(e) => {
                tracing::error!("mcp servers lock poisoned on write: {e}; recovering");
                self.servers.write().unwrap_or_else(|p| p.into_inner())
            }
        }
    }

    fn write_defaults(&self) -> std::sync::RwLockWriteGuard<'_, crate::config::McpDefaults> {
        match self.defaults.write() {
            Ok(g) => g,
            Err(e) => {
                tracing::error!("mcp defaults lock poisoned on write: {e}; recovering");
                self.defaults.write().unwrap_or_else(|p| p.into_inner())
            }
        }
    }

    pub fn has_servers(&self) -> bool {
        self.read_servers().iter().any(|s| s.enabled)
    }

    pub async fn connect_eager(&self) {
        let servers = self.read_servers().clone();
        let default_startup = self.read_defaults().startup;
        for server in servers {
            if !server.enabled {
                continue;
            }
            let startup = server.startup.unwrap_or(default_startup);
            if startup == McpStartup::Eager {
                let _ = self.ensure_connected(&server.id).await;
            }
        }
    }

    pub async fn status_snapshot(&self) -> Vec<McpServerStatus> {
        let status = self.status.read().await;
        let mut out: Vec<McpServerStatus> = status.values().cloned().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Configured `skills:` for an MCP server (empty when unknown or unset).
    pub fn server_skills(&self, server_id: &str) -> Vec<String> {
        self.read_servers()
            .iter()
            .find(|s| s.id == server_id)
            .map(|s| s.skills.clone())
            .unwrap_or_default()
    }

    /// Resolve federated tool name → server id (registry, then expose prefix).
    pub async fn server_id_for_tool(&self, global_name: &str) -> Option<String> {
        if let Some(entry) = self.resolve_entry(global_name).await {
            return Some(entry.server_id);
        }
        self.server_id_for_tool_by_prefix(global_name)
    }

    fn server_id_for_tool_by_prefix(&self, global_name: &str) -> Option<String> {
        self.read_servers()
            .iter()
            .filter(|s| s.enabled)
            .find(|s| global_name.starts_with(&expose_prefix(s)))
            .map(|s| s.id.clone())
    }

    pub async fn registry_entries_async(&self) -> Vec<GlobalToolEntry> {
        self.registry.read().await.entries().cloned().collect()
    }

    pub async fn is_mcp_tool_async(&self, name: &str) -> bool {
        self.registry.read().await.contains(name)
    }

    pub async fn is_mcp_mutating(&self, name: &str) -> bool {
        self.registry
            .read()
            .await
            .resolve(name)
            .is_some_and(|e| e.mutating)
    }

    pub async fn resolve_entry(&self, global_name: &str) -> Option<GlobalToolEntry> {
        self.registry.read().await.resolve(global_name).cloned()
    }

    pub async fn server_mutating_policy(&self, global_name: &str) -> Option<McpMutatingPolicy> {
        let entry = self.resolve_entry(global_name).await?;
        let servers = self.read_servers();
        servers
            .iter()
            .find(|s| s.id == entry.server_id)
            .map(|s| s.approval.mutating)
    }

    pub async fn call_global_tool(
        &self,
        global_name: &str,
        args: Value,
        cancel: McpCancel,
    ) -> Result<String> {
        self.call_global_tool_inner(global_name, args, false, cancel)
            .await
    }

    pub async fn call_global_tool_approved(
        &self,
        global_name: &str,
        args: Value,
    ) -> Result<String> {
        self.call_global_tool_inner(global_name, args, true, None)
            .await
    }

    async fn call_global_tool_inner(
        &self,
        global_name: &str,
        args: Value,
        approved: bool,
        cancel: McpCancel,
    ) -> Result<String> {
        let entry = self
            .registry
            .read()
            .await
            .resolve(global_name)
            .cloned()
            .ok_or_else(|| {
                CoworkerError::Workflow(format!("unknown federated tool {global_name:?}"))
            })?;
        if entry.mutating && !approved {
            return Err(CoworkerError::Workflow(format!(
                "{global_name} is a mutating MCP tool — approval required"
            )));
        }
        check_workflow_mcp_allowed(global_name, entry.mutating)?;
        self.ensure_connected(&entry.server_id).await?;
        let timeout_secs = self.server_timeout_for_id(&entry.server_id).await;
        let started = Instant::now();
        let is_stdio = self
            .sessions
            .read()
            .await
            .get(&entry.server_id)
            .is_some_and(|s| s.client.is_stdio());
        let result = {
            let mut sessions = self.sessions.write().await;
            let session = sessions.get_mut(&entry.server_id).ok_or_else(|| {
                CoworkerError::Other(anyhow::anyhow!("mcp session {:?} missing", entry.server_id))
            })?;
            session
                .client
                .call_tool(&entry.remote_name, args, timeout_secs, cancel.clone())
                .await
        };
        let elapsed = started.elapsed().as_millis();
        match result {
            Ok(text) => {
                tracing::info!(
                    "mcp.rpc server={} tool={} ms={elapsed}",
                    entry.server_id,
                    entry.remote_name
                );
                self.set_rpc_ok(&entry.server_id, elapsed).await;
                Ok(truncate_tool_output(
                    &text,
                    self.read_defaults().max_output_chars,
                ))
            }
            Err(e) => {
                if is_cancelled_error(&e) {
                    tracing::info!(
                        "mcp.rpc server={} tool={} cancelled after {elapsed}ms",
                        entry.server_id,
                        entry.remote_name
                    );
                    if is_stdio {
                        self.disconnect_server(&entry.server_id).await;
                    }
                    return Err(e);
                }
                tracing::warn!(
                    "mcp.rpc server={} tool={} ms={elapsed} err={e}",
                    entry.server_id,
                    entry.remote_name
                );
                self.set_rpc_ok(&entry.server_id, elapsed).await;
                Err(mcp_tool_rpc_error(global_name, &e))
            }
        }
    }

    /// Read `mcp+{server_id}://...` resources via MCP `resources/read`.
    pub async fn read_federated_resource(&self, uri: &str, cancel: McpCancel) -> Result<String> {
        check_workflow_mcp_allowed("resource_read", false)?;
        let rest = uri
            .strip_prefix("mcp+")
            .ok_or_else(|| CoworkerError::Workflow(format!("unsupported resource URI {uri:?}")))?;
        let (server_id, resource_uri) = rest
            .split_once("://")
            .ok_or_else(|| CoworkerError::Workflow(format!("invalid mcp resource URI {uri:?}")))?;
        self.ensure_connected(server_id).await?;
        let timeout_secs = self.server_timeout_for_id(server_id).await;
        let started = Instant::now();
        let is_stdio = self
            .sessions
            .read()
            .await
            .get(server_id)
            .is_some_and(|s| s.client.is_stdio());
        let result = {
            let mut sessions = self.sessions.write().await;
            let session = sessions.get_mut(server_id).ok_or_else(|| {
                CoworkerError::Other(anyhow::anyhow!("mcp session {server_id:?} missing"))
            })?;
            session
                .client
                .read_resource(resource_uri, timeout_secs, cancel.clone())
                .await
        };
        let elapsed = started.elapsed().as_millis();
        match result {
            Ok(text) => {
                tracing::info!("mcp.rpc server={server_id} tool=resources/read ms={elapsed}");
                self.set_rpc_ok(server_id, elapsed).await;
                Ok(truncate_tool_output(
                    &text,
                    self.read_defaults().max_output_chars,
                ))
            }
            Err(e) => {
                if is_cancelled_error(&e) {
                    tracing::info!(
                        "mcp.rpc server={server_id} tool=resources/read cancelled after {elapsed}ms"
                    );
                    if is_stdio {
                        self.disconnect_server(server_id).await;
                    }
                    return Err(e);
                }
                tracing::warn!(
                    "mcp.rpc server={server_id} tool=resources/read ms={elapsed} err={e}"
                );
                self.set_rpc_ok(server_id, elapsed).await;
                Err(mcp_tool_rpc_error(
                    &format!("mcp+{server_id}://{resource_uri}"),
                    &e,
                ))
            }
        }
    }

    pub async fn reload_from_config(&self, config: McpConfig) {
        self.sessions.write().await.clear();
        *self.registry.write().await = GlobalToolRegistry::default();
        *self.write_defaults() = config.defaults;
        *self.write_servers() = config.servers.clone();
        *self.status.write().await = initial_status_map(&config.servers);
        self.connect_eager().await;
    }

    pub async fn server_sections_async(&self) -> Vec<String> {
        let sessions = self.sessions.read().await;
        let mut out = Vec::new();
        for (id, session) in sessions.iter() {
            let mut section = format!("## mcp:{id} ({} tools)\n", session.tools.len());
            for tool in &session.tools {
                section.push_str(&format!(
                    "- {} — {}\n",
                    tool.remote_name,
                    brief(&tool.description)
                ));
            }
            out.push(section.trim_end().to_string());
        }
        out
    }

    pub async fn describe_tool_async(&self, global_name: &str) -> Option<String> {
        let entry = self.registry.read().await.resolve(global_name)?.clone();
        self.ensure_connected(&entry.server_id).await.ok()?;
        let sessions = self.sessions.read().await;
        let session = sessions.get(&entry.server_id)?;
        let tool = session
            .tools
            .iter()
            .find(|t| t.remote_name == entry.remote_name)?;
        Some(format!(
            "{global_name} (server: {})\n{}\n\nParameters (JSON Schema):\n{}",
            entry.server_id,
            tool.description,
            serde_json::to_string_pretty(&tool.input_schema).unwrap_or_else(|_| "{}".into())
        ))
    }

    async fn server_timeout_for_id(&self, server_id: &str) -> u64 {
        let default_secs = self.read_defaults().timeout_secs;
        self.read_servers()
            .iter()
            .find(|s| s.id == server_id)
            .map(|s| server_timeout(s, default_secs))
            .unwrap_or(default_secs)
    }

    async fn ensure_connected(&self, server_id: &str) -> Result<()> {
        if self.sessions.read().await.contains_key(server_id) {
            return Ok(());
        }
        let server = self
            .read_servers()
            .iter()
            .find(|s| s.id == server_id)
            .cloned()
            .ok_or_else(|| {
                CoworkerError::Other(anyhow::anyhow!("unknown mcp server {server_id:?}"))
            })?;
        if !server.enabled {
            return Err(CoworkerError::Other(anyhow::anyhow!(
                "mcp server {server_id:?} is disabled"
            )));
        }
        let default_startup = self.read_defaults().startup;
        let startup = server.startup.unwrap_or(default_startup);
        if startup == McpStartup::Disabled {
            return Err(CoworkerError::Other(anyhow::anyhow!(
                "mcp server {server_id:?} startup=disabled"
            )));
        }
        self.connect_server(&server).await
    }

    async fn disconnect_server(&self, server_id: &str) {
        if let Some(mut session) = self.sessions.write().await.remove(server_id) {
            session.client.abort_in_flight();
        }
        self.registry.write().await.remove_server(server_id);
        let mut status = self.status.write().await;
        if let Some(s) = status.get_mut(server_id) {
            s.connected = false;
            s.tool_count = 0;
            s.last_error = Some("cancelled".into());
        }
    }

    async fn connect_server(&self, server: &McpServerConfig) -> Result<()> {
        let started = Instant::now();
        let default_secs = self.read_defaults().timeout_secs;
        let timeout_secs = server_timeout(server, default_secs);
        let result: Result<(McpClient, Vec<McpToolDescriptor>)> = async {
            let mut client = McpClient::connect(server, timeout_secs).await?;
            let tools = client.list_tools(None).await?;
            Ok((client, tools))
        }
        .await;
        match result {
            Ok((client, tools)) => {
                let mut registry = self.registry.write().await;
                registry.remove_server(&server.id);
                registry.register_server(server, &tools)?;
                let tool_count = registry
                    .entries()
                    .filter(|e| e.server_id == server.id)
                    .count() as u32;
                self.sessions
                    .write()
                    .await
                    .insert(server.id.clone(), ConnectedServer { client, tools });
                self.set_connected(&server.id, tool_count, started.elapsed().as_millis())
                    .await;
                tracing::info!(
                    "mcp server {:?} connected ({tool_count} federated tools)",
                    server.id
                );
                Ok(())
            }
            Err(e) => {
                self.set_error(&server.id, &e.to_string()).await;
                Err(e)
            }
        }
    }

    async fn set_connected(&self, id: &str, tool_count: u32, ms: u128) {
        let mut status = self.status.write().await;
        if let Some(s) = status.get_mut(id) {
            s.connected = true;
            s.tool_count = tool_count;
            s.last_error = None;
            s.last_rpc_ms = Some(ms);
        }
    }

    async fn set_rpc_ok(&self, id: &str, ms: u128) {
        let mut status = self.status.write().await;
        if let Some(s) = status.get_mut(id) {
            s.last_rpc_ms = Some(ms);
            s.last_error = None;
        }
    }

    async fn set_error(&self, id: &str, message: &str) {
        let mut status = self.status.write().await;
        if let Some(s) = status.get_mut(id) {
            s.connected = false;
            s.tool_count = 0;
            s.last_error = Some(message.to_string());
        }
    }
}

pub async fn spawn_mcp_pool(config: &crate::config::Config) -> Arc<McpPool> {
    let pool = Arc::new(McpPool::new(config.mcp.clone()));
    if pool.has_servers() {
        pool.connect_eager().await;
    }
    pool
}

fn server_timeout(server: &McpServerConfig, default_secs: u64) -> u64 {
    server.timeout_secs.unwrap_or(default_secs)
}

fn expose_prefix(server: &McpServerConfig) -> String {
    server
        .expose
        .prefix
        .clone()
        .unwrap_or_else(|| format!("{}_", server.id))
}

fn initial_status_map(servers: &[McpServerConfig]) -> HashMap<String, McpServerStatus> {
    servers
        .iter()
        .map(|s| {
            (
                s.id.clone(),
                McpServerStatus {
                    id: s.id.clone(),
                    connected: false,
                    tool_count: 0,
                    last_error: None,
                    last_rpc_ms: None,
                    prefix: expose_prefix(s),
                },
            )
        })
        .collect()
}

fn mcp_tool_rpc_error(tool_name: &str, err: &CoworkerError) -> CoworkerError {
    let msg = err.to_string();
    CoworkerError::Workflow(format!(
        "{}\n{}",
        crate::agent::harness_errors::harn_header(tool_name, "MCP_RPC_FAILED"),
        crate::agent::harness_errors::format_error_line(
            "MCP_RPC_FAILED",
            &msg,
            "Check MCP server status, tool_describe schema, and server logs"
        )
    ))
}

fn brief(desc: &str) -> String {
    if let Some(dot) = desc.find(". ") {
        desc[..dot + 1].to_string()
    } else if desc.len() > 96 {
        format!("{}…", &desc[..96])
    } else {
        desc.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::{json, Value};
    use tokio::sync::Mutex;

    use super::*;
    use crate::config::{
        McpApprovalConfig, McpConfig, McpDefaults, McpExposeConfig, McpServerConfig, McpTransport,
    };

    fn mock_http_server(url: &str) -> McpServerConfig {
        McpServerConfig {
            id: "mock".into(),
            enabled: true,
            transport: McpTransport::Http,
            command: None,
            args: vec![],
            env: HashMap::new(),
            url: Some(url.into()),
            headers: HashMap::new(),
            expose: McpExposeConfig {
                prefix: Some("mock_".into()),
                allowlist: vec!["ping".into(), "slow".into()],
                denylist: vec![],
            },
            approval: McpApprovalConfig::default(),
            startup: Some(McpStartup::Eager),
            timeout_secs: Some(5),
            skills: vec![],
        }
    }

    #[tokio::test]
    async fn parallel_readonly_mcp_tools() {
        let call_count = Arc::new(Mutex::new(0u32));
        let counter = Arc::clone(&call_count);
        let app = Router::new().route(
            "/mcp",
            post({
                let counter = Arc::clone(&counter);
                move |Json(body): Json<Value>| {
                    let counter = Arc::clone(&counter);
                    async move {
                        let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
                        let id = body.get("id").cloned();
                        if method == "tools/call" {
                            let mut n = counter.lock().await;
                            *n += 1;
                            let current = *n;
                            drop(n);
                            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                            return (
                                StatusCode::OK,
                                [(header::CONTENT_TYPE, "application/json")],
                                Json(json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "content": [{ "type": "text", "text": format!("ok-{current}") }],
                                        "isError": false
                                    }
                                })),
                            )
                                .into_response();
                        }
                        match method {
                            "initialize" => (
                                StatusCode::OK,
                                [(header::CONTENT_TYPE, "application/json")],
                                Json(json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "protocolVersion": "2024-11-05",
                                        "capabilities": {},
                                        "serverInfo": { "name": "mock", "version": "0" }
                                    }
                                })),
                            )
                                .into_response(),
                            "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
                            "tools/list" => (
                                StatusCode::OK,
                                [(header::CONTENT_TYPE, "application/json")],
                                Json(json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "tools": [
                                            {
                                                "name": "ping",
                                                "description": "ping",
                                                "inputSchema": { "type": "object" },
                                                "annotations": { "readOnlyHint": true }
                                            },
                                            {
                                                "name": "slow",
                                                "description": "slow",
                                                "inputSchema": { "type": "object" },
                                                "annotations": { "readOnlyHint": true }
                                            }
                                        ]
                                    }
                                })),
                            )
                                .into_response(),
                            _ => StatusCode::NOT_FOUND.into_response(),
                        }
                    }
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let url = format!("http://{addr}/mcp");
        let pool = McpPool::new(McpConfig {
            defaults: McpDefaults::default(),
            servers: vec![mock_http_server(&url)],
        });
        pool.connect_eager().await;

        let pool = Arc::new(pool);
        let (a, b) = tokio::join!(
            pool.call_global_tool("mock_ping", json!({}), None),
            pool.call_global_tool("mock_slow", json!({}), None)
        );
        assert!(a.unwrap().contains("ok-"));
        assert!(b.unwrap().contains("ok-"));
        assert!(*call_count.lock().await >= 2);
    }

    #[tokio::test]
    async fn http_cancel_aborts_slow_rpc() {
        use std::sync::atomic::{AtomicBool, Ordering};

        use crate::mcp::cancel::is_cancelled_error;

        let handler_done = Arc::new(AtomicBool::new(false));
        let done_flag = Arc::clone(&handler_done);
        let app = Router::new().route(
            "/mcp",
            post({
                move |Json(body): Json<Value>| {
                    let done_flag = Arc::clone(&done_flag);
                    async move {
                        let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
                        let id = body.get("id").cloned();
                        if method == "tools/call" {
                            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                            done_flag.store(true, Ordering::Relaxed);
                            return (
                                StatusCode::OK,
                                [(header::CONTENT_TYPE, "application/json")],
                                Json(json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "content": [{ "type": "text", "text": "late" }],
                                        "isError": false
                                    }
                                })),
                            )
                                .into_response();
                        }
                        match method {
                            "initialize" => (
                                StatusCode::OK,
                                [(header::CONTENT_TYPE, "application/json")],
                                Json(json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "protocolVersion": "2024-11-05",
                                        "capabilities": {},
                                        "serverInfo": { "name": "mock", "version": "0" }
                                    }
                                })),
                            )
                                .into_response(),
                            "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
                            "tools/list" => (
                                StatusCode::OK,
                                [(header::CONTENT_TYPE, "application/json")],
                                Json(json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "tools": [{
                                            "name": "slow",
                                            "description": "slow",
                                            "inputSchema": { "type": "object" },
                                            "annotations": { "readOnlyHint": true }
                                        }]
                                    }
                                })),
                            )
                                .into_response(),
                            _ => StatusCode::NOT_FOUND.into_response(),
                        }
                    }
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let url = format!("http://{addr}/mcp");
        let pool = Arc::new(McpPool::new(McpConfig {
            defaults: McpDefaults::default(),
            servers: vec![mock_http_server(&url)],
        }));
        pool.connect_eager().await;

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_flag = Arc::clone(&cancel);
        let pool_call = Arc::clone(&pool);
        let call_task = tokio::spawn(async move {
            pool_call
                .call_global_tool("mock_slow", json!({}), Some(cancel_flag))
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        cancel.store(true, Ordering::Relaxed);
        let result = call_task.await.expect("join");
        assert!(result.is_err());
        assert!(is_cancelled_error(&result.unwrap_err()));
        assert!(
            !handler_done.load(Ordering::Relaxed),
            "server handler should not finish after cancel"
        );
    }
}
