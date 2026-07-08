//! React UI routes — serves the Vite-built SPA from `../../web-ui/dist/`.
//!
//! With **`embed-web-ui`** (release / CI): assets are compiled into the binary
//! via `_dist_manifest.rs` (`include_str!`).
//!
//! Without it (default dev `cargo build`): assets are read from disk at runtime
//! so frontend rebuilds do not force a Rust recompile.

#[cfg(not(feature = "embed-web-ui"))]
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::Path as AxumPath;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

/// Asset content embedded by the build script. Text assets (JS/CSS) are
/// stored as `&str`; binary assets (fonts/images, if any) as `&[u8]`.
///
/// Either variant may be uninhabited depending on what `build.rs` embedded
/// (e.g. when `../../web-ui/dist/` is absent the manifest is a stub and neither
/// variant is constructed), so both allow `dead_code`.
#[allow(dead_code)]
pub enum AssetContent {
    Text(&'static str),
    Binary(&'static [u8]),
}

#[cfg(feature = "embed-web-ui")]
include!("_dist_manifest.rs");

#[cfg(not(feature = "embed-web-ui"))]
mod disk_embed {
    #![allow(dead_code)]
    pub static INDEX_HTML: &str = "";
    pub static ASSETS: &[(&str, super::AssetContent)] = &[];
    pub static HAS_DIST: bool = false;
}

#[cfg(not(feature = "embed-web-ui"))]
fn web_dist_root() -> PathBuf {
    if let Ok(p) = std::env::var("COWORKER_WEB_DIST") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web-ui/dist")
}

#[cfg(not(feature = "embed-web-ui"))]
fn read_disk_file(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

/// Router for the React UI: `/` (index) + `/assets/{name}` (hashed chunks).
pub fn react_router() -> Router<()> {
    Router::new()
        .route("/", get(react_index))
        .route("/assets/{*name}", get(react_asset))
}

async fn react_index() -> Response {
    #[cfg(feature = "embed-web-ui")]
    {
        if !HAS_DIST || INDEX_HTML.is_empty() {
            react_unavailable()
        } else {
            (
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                )],
                INDEX_HTML,
            )
                .into_response()
        }
    }

    #[cfg(not(feature = "embed-web-ui"))]
    {
        let path = web_dist_root().join("index.html");
        match read_disk_file(&path) {
            Some(bytes) => (
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                )],
                String::from_utf8_lossy(&bytes).into_owned(),
            )
                .into_response(),
            None => react_unavailable(),
        }
    }
}

async fn react_asset(AxumPath(name): AxumPath<String>) -> Response {
    let key = format!("assets/{name}");

    #[cfg(feature = "embed-web-ui")]
    {
        let asset = ASSETS.iter().find(|(p, _)| *p == key);
        match asset {
            Some((_, AssetContent::Text(body))) => {
                let ct = mime_for(&name);
                (
                    [(header::CONTENT_TYPE, HeaderValue::from_static(ct))],
                    *body,
                )
                    .into_response()
            }
            Some((_, AssetContent::Binary(body))) => {
                let ct = mime_for(&name);
                (
                    [(header::CONTENT_TYPE, HeaderValue::from_static(ct))],
                    Body::from(*body),
                )
                    .into_response()
            }
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }

    #[cfg(not(feature = "embed-web-ui"))]
    {
        let path = web_dist_root().join(&key);
        let Some(bytes) = read_disk_file(&path) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let ct = mime_for(&name);
        if name.ends_with(".js") || name.ends_with(".css") {
            (
                [(header::CONTENT_TYPE, HeaderValue::from_static(ct))],
                String::from_utf8_lossy(&bytes).into_owned(),
            )
                .into_response()
        } else {
            (
                [(header::CONTENT_TYPE, HeaderValue::from_static(ct))],
                Body::from(bytes),
            )
                .into_response()
        }
    }
}

fn react_unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        "React UI not built. Run `cd web-ui && npm install && npm run build:fast` (dev serves dist/ from disk; release uses `--features embed-web-ui`).",
    )
        .into_response()
}

fn mime_for(name: &str) -> &'static str {
    if name.ends_with(".js") {
        "application/javascript"
    } else if name.ends_with(".css") {
        "text/css"
    } else if name.ends_with(".woff2") {
        "font/woff2"
    } else if name.ends_with(".png") {
        "image/png"
    } else if name.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}
