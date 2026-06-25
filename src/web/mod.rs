mod snapshot;

#[cfg(test)]
mod api_tests;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::agent::chat_loop::ChatProgress;
use crate::app::{
    apply_event, export_chat_transcript_markdown, hydrate_from_store, load_chat_session_ui,
    spawn_approval_decision, AppEvent, SharedState, Tab,
};
use crate::engine::Engine;
use crate::error::Result;
use crate::store::Store;

use snapshot::{build_chat_patch, build_live_patch, build_snapshot};

pub struct WebRuntime {
    pub state: SharedState,
    pub engine: Arc<Engine>,
    #[allow(dead_code)]
    pub store: Arc<dyn Store>,
    pub snap_tx: broadcast::Sender<String>,
}

pub async fn run(
    bind: SocketAddr,
    state: SharedState,
    engine: Arc<Engine>,
    store: Arc<dyn Store>,
    events_rx: broadcast::Receiver<AppEvent>,
    attach: bool,
    auth_token: Option<String>,
) -> Result<()> {
    let (snap_tx, _) = broadcast::channel::<String>(256);
    let runtime = Arc::new(WebRuntime {
        state: state.clone(),
        engine: engine.clone(),
        store: store.clone(),
        snap_tx: snap_tx.clone(),
    });

    spawn_event_loop(
        state.clone(),
        store.clone(),
        events_rx,
        snap_tx.clone(),
        attach,
    );

    let app = build_router(runtime, auth_token);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| crate::error::CoworkerError::Workflow(format!("bind {bind}: {e}")))?;
    tracing::info!("WebUI at http://{bind}");
    axum::serve(listener, app)
        .await
        .map_err(|e| crate::error::CoworkerError::Workflow(format!("web server: {e}")))?;
    Ok(())
}

pub(crate) fn build_router(runtime: Arc<WebRuntime>, auth_token: Option<String>) -> Router {
    let protected = Router::new()
        .route("/api/state", get(api_state))
        .route("/api/tab/{tab}", post(api_set_tab))
        .route("/api/chat", post(api_chat))
        .route("/api/chat/cancel", post(api_chat_cancel))
        .route("/api/chat/clear", post(api_chat_clear))
        .route("/api/chat/sessions", get(api_list_chat_sessions))
        .route("/api/chat/sessions/new", post(api_new_chat_session))
        .route("/api/chat/sessions/{id}", post(api_load_chat_session))
        .route("/api/chat/context", post(api_toggle_context))
        .route("/api/chat/export", get(api_chat_export))
        .route("/api/approvals/{id}", post(api_approval))
        .route("/api/approvals/history", get(api_approval_history))
        .route("/api/workflows/{id}", post(api_run_workflow))
        .route("/api/store/refresh", post(api_refresh_store))
        .route("/api/prs/filter", post(api_prs_filter))
        .route("/api/prs/sort", post(api_prs_sort))
        .route("/api/prs/{index}/select", post(api_prs_select))
        .route("/api/prs/{index}/triage", post(api_prs_triage))
        .route("/api/prs/{index}/overview", post(api_pr_overview))
        .route("/api/logs/filter", post(api_logs_filter))
        .route("/api/digest/{index}/select", post(api_digest_select))
        .route("/api/config/probe", post(api_config_probe))
        .route("/ws", get(ws_handler));

    let protected = if let Some(token) = effective_auth_token(auth_token.as_ref()) {
        tracing::info!("Web UI bearer auth enabled for /api/* and /ws");
        protected.layer(middleware::from_fn_with_state(
            Arc::from(token),
            require_bearer_auth,
        ))
    } else {
        protected
    };

    Router::new()
        .route("/", get(index))
        .route("/markdown.js", get(markdown_js))
        .route("/approvals.js", get(approvals_js))
        .route("/app.js", get(js))
        .route("/style.css", get(css))
        .route("/api/health", get(api_health))
        .merge(protected)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(runtime)
}

fn effective_auth_token(token: Option<&String>) -> Option<&str> {
    token
        .map(String::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

fn bearer_matches(headers: &axum::http::HeaderMap, expected: &str) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|token| token == expected)
}

async fn require_bearer_auth(
    State(expected): State<Arc<str>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if bearer_matches(req.headers(), expected.as_ref()) {
        next.run(req).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

fn spawn_event_loop(
    state: SharedState,
    store: Arc<dyn Store>,
    events_rx: broadcast::Receiver<AppEvent>,
    snap_tx: broadcast::Sender<String>,
    attach: bool,
) {
    tokio::spawn(async move {
        use tokio::time::{interval, MissedTickBehavior};

        let mut events_rx = events_rx;
        let mut poll = interval(std::time::Duration::from_secs(2));
        let mut arm_poll = interval(std::time::Duration::from_millis(100));
        arm_poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut live_poll = interval(std::time::Duration::from_millis(100));
        live_poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut live_dirty = false;
        let mut chat_dirty = false;
        loop {
            tokio::select! {
                ev = events_rx.recv() => {
                    match ev {
                        Ok(ev) => {
                            match event_snapshot_kind(&ev) {
                                SnapshotKind::Full => {
                                    apply_event(&state, ev).await;
                                    live_dirty = false;
                                    chat_dirty = false;
                                    publish_snapshot(&state, &snap_tx).await;
                                }
                                SnapshotKind::Chat => {
                                    apply_event(&state, ev).await;
                                    live_dirty = false;
                                    chat_dirty = true;
                                }
                                SnapshotKind::Live => {
                                    apply_event(&state, ev).await;
                                    live_dirty = true;
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = live_poll.tick(), if chat_dirty || live_dirty => {
                    if chat_dirty {
                        chat_dirty = false;
                        live_dirty = false;
                        publish_chat_patch(&state, &snap_tx).await;
                    } else {
                        live_dirty = false;
                        publish_live_patch(&state, &snap_tx).await;
                    }
                }
                _ = poll.tick(), if attach => {
                    if hydrate_from_store(&state, store.as_ref()).await.is_ok() {
                        live_dirty = false;
                        chat_dirty = false;
                        publish_snapshot(&state, &snap_tx).await;
                    }
                }
                _ = arm_poll.tick() => {
                    let waiting_arm = {
                        let s = state.read().await;
                        s.approval_dialog
                            .as_ref()
                            .is_some_and(|d| !d.deciding && !d.approve_armed())
                    };
                    if waiting_arm {
                        live_dirty = false;
                        chat_dirty = false;
                        publish_snapshot(&state, &snap_tx).await;
                    }
                }
            }
        }
    });
}

enum SnapshotKind {
    Full,
    Chat,
    Live,
}

fn event_snapshot_kind(ev: &AppEvent) -> SnapshotKind {
    match ev {
        AppEvent::ChatProgress(p) if chat_progress_is_live_only(p) => SnapshotKind::Live,
        AppEvent::ChatProgress(_) => SnapshotKind::Chat,
        AppEvent::ChatReply => SnapshotKind::Chat,
        _ => SnapshotKind::Full,
    }
}

fn chat_progress_is_live_only(p: &ChatProgress) -> bool {
    matches!(
        p,
        ChatProgress::AssistantPartial { .. }
            | ChatProgress::ReasoningPartial { .. }
            | ChatProgress::ToolProgress { .. }
            | ChatProgress::TurnThinking { .. }
            | ChatProgress::ReasoningCompressing
            | ChatProgress::ToolPending { .. }
            | ChatProgress::ActivityFlow { .. }
            | ChatProgress::ActivityFlowClear
    )
}

async fn publish_live_patch(state: &SharedState, snap_tx: &broadcast::Sender<String>) {
    let patch = build_live_patch(state).await;
    if let Ok(json) = serde_json::to_string(&patch) {
        let _ = snap_tx.send(json);
    }
}

async fn publish_chat_patch(state: &SharedState, snap_tx: &broadcast::Sender<String>) {
    let patch = build_chat_patch(state).await;
    if let Ok(json) = serde_json::to_string(&patch) {
        let _ = snap_tx.send(json);
    }
}

async fn publish_snapshot(state: &SharedState, snap_tx: &broadcast::Sender<String>) {
    let snap = build_snapshot(state).await;
    if let Ok(json) = serde_json::to_string(&snap) {
        let _ = snap_tx.send(json);
    }
}

async fn index() -> Html<&'static str> {
    Html(include_str!("static/index.html"))
}

async fn markdown_js() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        include_str!("static/markdown.js"),
    )
}

async fn approvals_js() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        include_str!("static/approvals.js"),
    )
}

async fn js() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        include_str!("static/app.js"),
    )
}

async fn css() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css")],
        include_str!("static/style.css"),
    )
}

async fn api_state(State(rt): State<Arc<WebRuntime>>) -> Json<snapshot::WebSnapshot> {
    Json(build_snapshot(&rt.state).await)
}

async fn api_set_tab(State(rt): State<Arc<WebRuntime>>, Path(tab): Path<String>) -> StatusCode {
    {
        let mut s = rt.state.write().await;
        s.tab = match tab.as_str() {
            "chat" if s.config.chat.enabled => Tab::Chat,
            "dashboard" => Tab::Dashboard,
            "prs" => Tab::Prs,
            "approvals" => Tab::Approvals,
            "logs" => Tab::Logs,
            "config" => Tab::Config,
            _ => return StatusCode::BAD_REQUEST,
        };
    }
    publish_snapshot(&rt.state, &rt.snap_tx).await;
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
struct ChatBody {
    message: String,
}

async fn api_chat(State(rt): State<Arc<WebRuntime>>, Json(body): Json<ChatBody>) -> StatusCode {
    let msg = body.message.trim().to_string();
    if msg.is_empty() {
        return StatusCode::BAD_REQUEST;
    }
    if msg == "/clear" {
        reset_web_chat_session(&rt).await;
        return StatusCode::NO_CONTENT;
    }
    if msg == "/new" {
        reset_web_chat_session(&rt).await;
        return StatusCode::NO_CONTENT;
    }
    if msg == "/help" {
        let mut s = rt.state.write().await;
        s.status = "Slash: /clear /new — reset transcript + LLM context; /sessions /session <id> — history; /export [path] — markdown; /approve /deny — pending approval".into();
        drop(s);
        publish_snapshot(&rt.state, &rt.snap_tx).await;
        return StatusCode::NO_CONTENT;
    }
    if msg == "/approve" || msg == "/deny" {
        let approve = msg == "/approve";
        let id = {
            let s = rt.state.read().await;
            s.approval_dialog.as_ref().map(|d| d.id)
        };
        if let Some(id) = id {
            spawn_approval_decision(&rt.state, &rt.engine, id, approve).await;
            publish_snapshot(&rt.state, &rt.snap_tx).await;
        }
        return StatusCode::NO_CONTENT;
    }
    let session_id = {
        let s = rt.state.read().await;
        if s.chat_busy || !s.config.chat.enabled {
            return StatusCode::CONFLICT;
        }
        s.chat_session_id
    };
    let engine = Arc::clone(&rt.engine);
    let state = rt.state.clone();
    let snap_tx = rt.snap_tx.clone();
    tokio::spawn(async move {
        let _ = engine.run_chat(session_id, &msg).await;
        publish_snapshot(&state, &snap_tx).await;
    });
    StatusCode::ACCEPTED
}

async fn api_chat_cancel(State(rt): State<Arc<WebRuntime>>) -> StatusCode {
    rt.engine.request_chat_cancel();
    StatusCode::NO_CONTENT
}

async fn api_chat_clear(State(rt): State<Arc<WebRuntime>>) -> StatusCode {
    {
        let mut s = rt.state.write().await;
        s.reset_chat_session();
        s.status = "chat cleared".into();
        drop(s);
        publish_snapshot(&rt.state, &rt.snap_tx).await;
    }
    StatusCode::NO_CONTENT
}

#[derive(Serialize)]
struct ChatSessionItem {
    id: String,
    title: String,
    created_at: String,
}

async fn api_list_chat_sessions(
    State(rt): State<Arc<WebRuntime>>,
) -> std::result::Result<Json<Vec<ChatSessionItem>>, StatusCode> {
    let sessions = rt
        .store
        .list_chat_sessions(30)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        sessions
            .into_iter()
            .map(|s| ChatSessionItem {
                id: s.id.to_string(),
                title: s.title,
                created_at: s.created_at.format("%m-%d %H:%M").to_string(),
            })
            .collect(),
    ))
}

async fn api_new_chat_session(State(rt): State<Arc<WebRuntime>>) -> StatusCode {
    reset_web_chat_session(&rt).await;
    StatusCode::NO_CONTENT
}

async fn api_load_chat_session(
    State(rt): State<Arc<WebRuntime>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if rt.state.read().await.chat_busy {
        return StatusCode::CONFLICT;
    }
    {
        let mut s = rt.state.write().await;
        if load_chat_session_ui(&mut s, rt.store.as_ref(), id)
            .await
            .is_err()
        {
            return StatusCode::NOT_FOUND;
        }
        s.status = format!("loaded session {id}");
    }
    publish_snapshot(&rt.state, &rt.snap_tx).await;
    StatusCode::NO_CONTENT
}

async fn reset_web_chat_session(rt: &Arc<WebRuntime>) {
    let mut s = rt.state.write().await;
    s.reset_chat_session();
    s.status = "new chat session".into();
    drop(s);
    publish_snapshot(&rt.state, &rt.snap_tx).await;
}

#[derive(Deserialize)]
struct ContextToggle {
    visible: bool,
}

async fn api_toggle_context(
    State(rt): State<Arc<WebRuntime>>,
    Json(body): Json<ContextToggle>,
) -> StatusCode {
    let mut s = rt.state.write().await;
    s.chat_context_visible = body.visible;
    drop(s);
    publish_snapshot(&rt.state, &rt.snap_tx).await;
    StatusCode::NO_CONTENT
}

async fn api_chat_export(State(rt): State<Arc<WebRuntime>>) -> impl IntoResponse {
    let s = rt.state.read().await;
    let md = export_chat_transcript_markdown(&s);
    (
        [
            (
                axum::http::header::CONTENT_TYPE,
                "text/markdown; charset=utf-8",
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=\"chat-transcript.md\"",
            ),
        ],
        md,
    )
}

#[derive(Deserialize)]
struct ApprovalBody {
    approve: bool,
}

#[derive(Deserialize)]
struct ApprovalHistoryQuery {
    #[serde(default = "default_approval_history_limit")]
    limit: usize,
}

fn default_approval_history_limit() -> usize {
    50
}

fn approval_to_json(a: &crate::store::Approval) -> Value {
    json!({
        "id": a.id,
        "kind": format!("{:?}", a.kind),
        "description": a.description,
        "created_at": a.created_at.to_rfc3339(),
        "decided_at": a.decided_at.map(|t| t.to_rfc3339()),
        "repo": a.repo,
        "pr_number": a.pr_number,
        "run_id": a.run_id,
        "target_branch": a.target_branch,
        "status": format!("{:?}", a.status),
        "comment_body": a.comment_body,
        "issue_number": a.issue_number,
        "label": a.label,
    })
}

async fn api_approval_history(
    State(rt): State<Arc<WebRuntime>>,
    Query(q): Query<ApprovalHistoryQuery>,
) -> std::result::Result<Json<Vec<Value>>, StatusCode> {
    let limit = q.limit.clamp(1, 200);
    let items = rt
        .store
        .list_approval_history(limit)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(items.iter().map(approval_to_json).collect()))
}

async fn api_approval(
    State(rt): State<Arc<WebRuntime>>,
    Path(id): Path<Uuid>,
    Json(body): Json<ApprovalBody>,
) -> StatusCode {
    spawn_approval_decision(&rt.state, &rt.engine, id, body.approve).await;
    publish_snapshot(&rt.state, &rt.snap_tx).await;
    StatusCode::ACCEPTED
}

async fn api_run_workflow(State(rt): State<Arc<WebRuntime>>, Path(id): Path<String>) -> StatusCode {
    let engine = Arc::clone(&rt.engine);
    let state = rt.state.clone();
    let snap_tx = rt.snap_tx.clone();
    tokio::spawn(async move {
        let _ = engine.run_workflow(&id).await;
        publish_snapshot(&state, &snap_tx).await;
    });
    StatusCode::ACCEPTED
}

async fn api_refresh_store(State(rt): State<Arc<WebRuntime>>) -> StatusCode {
    if rt.engine.refresh_store().await.is_ok() {
        publish_snapshot(&rt.state, &rt.snap_tx).await;
        StatusCode::NO_CONTENT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

async fn api_pr_overview(
    State(rt): State<Arc<WebRuntime>>,
    Path(index): Path<usize>,
) -> StatusCode {
    let (repo, number) = {
        let s = rt.state.read().await;
        let filtered = s.sorted_filtered_prs();
        let Some(p) = filtered.get(index) else {
            return StatusCode::NOT_FOUND;
        };
        (p.repo.clone(), p.number)
    };
    let engine = Arc::clone(&rt.engine);
    let state = rt.state.clone();
    let snap_tx = rt.snap_tx.clone();
    tokio::spawn(async move {
        engine.fetch_pr_overview(repo, number).await;
        publish_snapshot(&state, &snap_tx).await;
    });
    StatusCode::ACCEPTED
}

async fn api_prs_filter(State(rt): State<Arc<WebRuntime>>) -> StatusCode {
    {
        let mut s = rt.state.write().await;
        s.pr_filter = s.pr_filter.next();
        s.status = format!("PR filter: {}", s.pr_filter.label());
    }
    publish_snapshot(&rt.state, &rt.snap_tx).await;
    StatusCode::NO_CONTENT
}

async fn api_prs_sort(State(rt): State<Arc<WebRuntime>>) -> StatusCode {
    {
        let mut s = rt.state.write().await;
        s.pr_sort = s.pr_sort.next();
        s.status = format!("PR sort: {}", s.pr_sort.label());
    }
    publish_snapshot(&rt.state, &rt.snap_tx).await;
    StatusCode::NO_CONTENT
}

async fn api_prs_select(State(rt): State<Arc<WebRuntime>>, Path(index): Path<usize>) -> StatusCode {
    {
        let mut s = rt.state.write().await;
        if index >= s.sorted_filtered_prs().len() {
            return StatusCode::NOT_FOUND;
        }
        s.selected_index = index;
    }
    publish_snapshot(&rt.state, &rt.snap_tx).await;
    StatusCode::NO_CONTENT
}

async fn api_prs_triage(State(rt): State<Arc<WebRuntime>>, Path(index): Path<usize>) -> StatusCode {
    let (repo, number) = {
        let s = rt.state.read().await;
        let filtered = s.sorted_filtered_prs();
        let Some(p) = filtered.get(index) else {
            return StatusCode::NOT_FOUND;
        };
        (p.repo.clone(), p.number)
    };
    rt.engine.spawn_triage_pr(repo, number);
    publish_snapshot(&rt.state, &rt.snap_tx).await;
    StatusCode::ACCEPTED
}

async fn api_logs_filter(State(rt): State<Arc<WebRuntime>>) -> StatusCode {
    {
        let mut s = rt.state.write().await;
        s.log_filter = s.log_filter.next();
        s.status = format!("Log filter: {}", s.log_filter.label());
    }
    publish_snapshot(&rt.state, &rt.snap_tx).await;
    StatusCode::NO_CONTENT
}

async fn api_digest_select(
    State(rt): State<Arc<WebRuntime>>,
    Path(index): Path<usize>,
) -> StatusCode {
    {
        let mut s = rt.state.write().await;
        if index >= s.digest_history.len() {
            return StatusCode::NOT_FOUND;
        }
        s.selected_index = index;
    }
    publish_snapshot(&rt.state, &rt.snap_tx).await;
    StatusCode::NO_CONTENT
}

async fn api_config_probe(State(rt): State<Arc<WebRuntime>>) -> StatusCode {
    rt.engine.refresh_connectivity_probes().await;
    publish_snapshot(&rt.state, &rt.snap_tx).await;
    StatusCode::NO_CONTENT
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    gh: String,
    llm: String,
    mcp: Vec<Value>,
}

fn format_probe_status(ok: bool, latency_ms: Option<u128>) -> String {
    if !ok {
        "offline".to_string()
    } else {
        latency_ms
            .map(|ms| format!("{ms}ms"))
            .unwrap_or_else(|| "ok".to_string())
    }
}

async fn api_health(State(rt): State<Arc<WebRuntime>>) -> Json<HealthResponse> {
    let s = rt.state.read().await;
    let gh = format_probe_status(s.github_ok, s.github_latency_ms);
    let llm = format_probe_status(s.llm_ok, s.llm_latency_ms);
    let mcp = s
        .mcp_servers
        .iter()
        .map(|server| {
            json!({
                "id": server.id,
                "connected": server.connected,
                "tool_count": server.tool_count,
                "last_error": server.last_error,
                "last_rpc_ms": server.last_rpc_ms,
                "prefix": server.prefix,
            })
        })
        .collect();
    Json(HealthResponse {
        ok: s.github_ok && s.llm_ok,
        gh,
        llm,
        mcp,
    })
}

async fn ws_handler(ws: WebSocketUpgrade, State(rt): State<Arc<WebRuntime>>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, rt))
}

async fn handle_socket(socket: WebSocket, rt: Arc<WebRuntime>) {
    let (mut sender, mut receiver) = socket.split();
    let mut snap_rx = rt.snap_tx.subscribe();

    let initial = build_snapshot(&rt.state).await;
    if let Ok(text) = serde_json::to_string(&initial) {
        if sender.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            msg = snap_rx.recv() => {
                match msg {
                    Ok(text) => {
                        if sender.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
            incoming = receiver.next() => {
                #[allow(clippy::collapsible_match)]
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(p))) => {
                        if sender.send(Message::Pong(p)).await.is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod auth_tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn effective_auth_token_rejects_blank() {
        assert_eq!(effective_auth_token(None), None);
        assert_eq!(effective_auth_token(Some(&String::new())), None);
        assert_eq!(effective_auth_token(Some(&"   ".into())), None);
        assert_eq!(effective_auth_token(Some(&"secret".into())), Some("secret"));
        assert_eq!(effective_auth_token(Some(&"  tok  ".into())), Some("tok"));
    }

    #[test]
    fn bearer_matches_header() {
        let mut headers = axum::http::HeaderMap::new();
        assert!(!bearer_matches(&headers, "tok"));

        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer tok"));
        assert!(bearer_matches(&headers, "tok"));
        assert!(!bearer_matches(&headers, "wrong"));
        assert!(!bearer_matches(&headers, "tok "));
    }
}
