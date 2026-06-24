use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::error::CoworkerError;

pub const CHAT_CANCELLED: &str = "chat cancelled";

pub type McpCancel = Option<Arc<AtomicBool>>;

pub fn cancelled_error() -> CoworkerError {
    CoworkerError::Workflow(CHAT_CANCELLED.into())
}

pub fn is_cancelled(cancel: &McpCancel) -> bool {
    cancel
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::Relaxed))
}

pub fn is_cancelled_error(err: &CoworkerError) -> bool {
    matches!(err, CoworkerError::Workflow(msg) if msg == CHAT_CANCELLED)
}

pub async fn wait_until_cancelled(cancel: &McpCancel) {
    let Some(flag) = cancel else {
        std::future::pending::<()>().await;
        return;
    };
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
