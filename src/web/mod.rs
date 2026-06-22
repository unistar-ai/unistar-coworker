mod snapshot;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::app::{
    apply_event, hydrate_from_store, spawn_approval_decision, AppEvent, SharedState, Tab,
};
use crate::agent::chat_loop::ChatProgress;
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
) -> Result<()> {
    let (snap_tx, _) = broadcast::channel::<String>(256);
    let runtime = Arc::new(WebRuntime {
        state: state.clone(),
        engine: engine.clone(),
        store: store.clone(),
        snap_tx: snap_tx.clone(),
    });

    spawn_event_loop(state.clone(), store.clone(), events_rx, snap_tx.clone(), attach);

    let app = Router::new()
        .route("/", get(index))
        .route("/app.js", get(js))
        .route("/style.css", get(css))
        .route("/api/state", get(api_state))
        .route("/api/tab/{tab}", post(api_set_tab))
        .route("/api/chat", post(api_chat))
        .route("/api/chat/cancel", post(api_chat_cancel))
        .route("/api/chat/clear", post(api_chat_clear))
        .route("/api/chat/context", post(api_toggle_context))
        .route("/api/approvals/{id}", post(api_approval))
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
        .route("/ws", get(ws_handler))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(runtime);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| crate::error::CoworkerError::Workflow(format!("bind {bind}: {e}")))?;
    tracing::info!("WebUI at http://{bind}");
    axum::serve(listener, app)
        .await
        .map_err(|e| crate::error::CoworkerError::Workflow(format!("web server: {e}")))?;
    Ok(())
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

async fn api_set_tab(
    State(rt): State<Arc<WebRuntime>>,
    Path(tab): Path<String>,
) -> StatusCode {
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
        let mut s = rt.state.write().await;
        s.reset_chat_session();
        s.status = "chat cleared".into();
        drop(s);
        publish_snapshot(&rt.state, &rt.snap_tx).await;
        return StatusCode::NO_CONTENT;
    }
    if msg == "/new" {
        let mut s = rt.state.write().await;
        s.reset_chat_session();
        s.status = "new chat session".into();
        drop(s);
        publish_snapshot(&rt.state, &rt.snap_tx).await;
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
    let mut s = rt.state.write().await;
    s.reset_chat_session();
    s.status = "chat cleared".into();
    drop(s);
    publish_snapshot(&rt.state, &rt.snap_tx).await;
    StatusCode::NO_CONTENT
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

#[derive(Deserialize)]
struct ApprovalBody {
    approve: bool,
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

async fn api_run_workflow(
    State(rt): State<Arc<WebRuntime>>,
    Path(id): Path<String>,
) -> StatusCode {
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

async fn api_prs_select(
    State(rt): State<Arc<WebRuntime>>,
    Path(index): Path<usize>,
) -> StatusCode {
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

async fn api_prs_triage(
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

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(rt): State<Arc<WebRuntime>>,
) -> Response {
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
