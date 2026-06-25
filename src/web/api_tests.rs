use std::sync::Arc;

use axum::body::Body;
use axum::http::{header::AUTHORIZATION, Request, StatusCode};
use axum::Router;
use futures_util::StreamExt;
use http_body_util::BodyExt;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;

use crate::app::{event_channel, AppState, SharedState};
use crate::config::Config;
use crate::engine::Engine;
use crate::store::json::JsonStore;
use crate::store::{Approval, ApprovalKind, ApprovalStatus, Store};

use super::{build_router, WebRuntime};

const MINIMAL_CONFIG: &str = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
storage: { backend: json, path: ./data }
repos: [acme/widget]
"#;

const SNAPSHOT_KEYS: &[&str] = &[
    "tab",
    "tabs",
    "status",
    "engine_busy",
    "chat_enabled",
    "chat_busy",
    "prs",
    "approvals",
    "logs",
    "config_path",
    "repos",
    "llm_model",
    "github_ok",
    "llm_ok",
    "ui_theme",
];

async fn test_runtime() -> Arc<WebRuntime> {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(JsonStore::open(dir.path().to_path_buf()).expect("json store")) as Arc<dyn Store>;
    std::mem::forget(dir);
    let mut config = Config::load_from_str(MINIMAL_CONFIG).expect("config");
    config.finalize();
    let (events_tx, _) = event_channel();
    let state: SharedState = Arc::new(tokio::sync::RwLock::new(AppState::new(
        config.clone(),
        "test.yaml".into(),
    )));
    let engine = Arc::new(
        Engine::new(config, Arc::clone(&store), events_tx, Arc::clone(&state)).await,
    );
    let (snap_tx, _) = broadcast::channel(256);
    Arc::new(WebRuntime {
        state,
        engine,
        store,
        snap_tx,
    })
}

fn test_app(runtime: Arc<WebRuntime>, auth_token: Option<String>) -> Router {
    build_router(runtime, auth_token)
}

async fn get_json(app: Router, uri: &str, bearer: Option<&str>) -> (StatusCode, Value) {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(token) = bearer {
        builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .expect("request");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).expect("json body")
    };
    (status, json)
}

async fn get_text(app: Router, uri: &str, bearer: Option<&str>) -> (StatusCode, String) {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(token) = bearer {
        builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .expect("request");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let text = String::from_utf8(body.to_vec()).expect("utf8 body");
    (status, text)
}

#[tokio::test]
async fn api_chat_export_returns_markdown() {
    let runtime = test_runtime().await;
    let app = test_app(Arc::clone(&runtime), None);

    let (status, text) = get_text(app, "/api/chat/export", None).await;

    assert_eq!(status, StatusCode::OK);
    assert!(text.starts_with("# Chat transcript"));
}

#[tokio::test]
async fn api_health_returns_connectivity_json() {
    let runtime = test_runtime().await;
    let app = test_app(Arc::clone(&runtime), None);

    let (status, json) = get_json(app, "/api/health", None).await;

    assert_eq!(status, StatusCode::OK);
    assert!(json["ok"].is_boolean());
    assert!(json["gh"].is_string());
    assert!(json["llm"].is_string());
    assert!(json["mcp"].is_array());
}

#[tokio::test]
async fn api_health_unauthenticated_when_token_configured() {
    let runtime = test_runtime().await;
    let app = test_app(runtime, Some("health-secret".into()));

    let (status, json) = get_json(app, "/api/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("ok").is_some());
}

#[tokio::test]
async fn api_state_returns_snapshot_json() {
    let runtime = test_runtime().await;
    let app = test_app(Arc::clone(&runtime), None);

    let (status, json) = get_json(app, "/api/state", None).await;

    assert_eq!(status, StatusCode::OK);
    assert!(json.is_object(), "expected JSON object, got {json}");
    for key in SNAPSHOT_KEYS {
        assert!(
            json.get(key).is_some(),
            "snapshot missing key {key}: {json}"
        );
    }
    assert_eq!(json["status"], "ready");
    assert_eq!(json["repos"], serde_json::json!(["acme/widget"]));
}

#[tokio::test]
async fn api_state_bearer_auth_when_token_configured() {
    let runtime = test_runtime().await;
    let token = "test-secret";

    let app_no_auth = test_app(Arc::clone(&runtime), Some(token.into()));
    let (status, _) = get_json(app_no_auth, "/api/state", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let app_wrong = test_app(Arc::clone(&runtime), Some(token.into()));
    let (status, _) = get_json(app_wrong, "/api/state", Some("wrong")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let app_ok = test_app(Arc::clone(&runtime), Some(token.into()));
    let (status, json) = get_json(app_ok, "/api/state", Some(token)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("tab").is_some());
}

#[tokio::test]
async fn ws_first_message_is_snapshot_json() {
    let runtime = test_runtime().await;
    let app = test_app(runtime, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve");
    });

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("ws connect");
    let msg = ws.next().await.expect("ws message").expect("ws frame");
    let text = match msg {
        tokio_tungstenite::tungstenite::Message::Text(t) => t,
        other => panic!("expected text frame, got {other:?}"),
    };
    let json: Value = serde_json::from_str(&text).expect("snapshot json");
    for key in SNAPSHOT_KEYS {
        assert!(json.get(key).is_some(), "ws snapshot missing key {key}");
    }
}

#[tokio::test]
async fn ws_requires_bearer_when_token_configured() {
    let runtime = test_runtime().await;
    let token = "ws-secret";
    let app = test_app(runtime, Some(token.into()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve");
    });

    let unauth = format!("ws://{addr}/ws")
        .into_client_request()
        .expect("request");
    let err = tokio_tungstenite::connect_async(unauth).await;
    assert!(err.is_err(), "expected unauthorized ws upgrade");

    let mut authed = format!("ws://{addr}/ws")
        .into_client_request()
        .expect("request");
    authed.headers_mut().insert(
        AUTHORIZATION,
        format!("Bearer {token}").parse().expect("header"),
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(authed)
        .await
        .expect("authorized ws connect");
    let msg = ws.next().await.expect("ws message").expect("ws frame");
    assert!(
        matches!(msg, tokio_tungstenite::tungstenite::Message::Text(_)),
        "expected snapshot text frame"
    );
}

#[tokio::test]
async fn api_approval_history_returns_decided_items() {
    use chrono::Utc;

    let runtime = test_runtime().await;
    let approval = Approval {
        id: uuid::Uuid::new_v4(),
        kind: ApprovalKind::BashRun,
        repo: "acme/widget".into(),
        pr_number: None,
        run_id: None,
        target_branch: None,
        incident_id: None,
        description: "run ls".into(),
        status: ApprovalStatus::Pending,
        created_at: Utc::now(),
        decided_at: None,
        comment_body: Some(r#"{"command":"ls -la"}"#.into()),
        issue_number: None,
        label: None,
    };
    runtime
        .store
        .push_approval(&approval)
        .await
        .expect("push approval");
    runtime
        .store
        .decide_approval(&approval.id, true)
        .await
        .expect("decide approval");

    let app = test_app(Arc::clone(&runtime), None);
    let (status, json) = get_json(app, "/api/approvals/history?limit=10", None).await;

    assert_eq!(status, StatusCode::OK);
    let items = json.as_array().expect("history array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], approval.id.to_string());
    assert_eq!(items[0]["status"], "Approved");
    assert_eq!(items[0]["kind"], "BashRun");
    assert!(items[0]["decided_at"].is_string());
}
