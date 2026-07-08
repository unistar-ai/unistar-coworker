use serde_json::Value;

use crate::error::{CoworkerError, Result};

pub fn require_str(args: &Value, key: &str) -> Result<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CoworkerError::Workflow(format!("missing required parameter: {key}")))
}

pub fn optional_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn require_u32(args: &Value, key: &str) -> Result<u32> {
    if let Some(n) = args.get(key).and_then(|v| v.as_u64()) {
        return Ok(n as u32);
    }
    if let Some(s) = args.get(key).and_then(|v| v.as_str()) {
        if let Ok(n) = s.parse::<u32>() {
            return Ok(n);
        }
    }
    Err(CoworkerError::Workflow(format!("missing or invalid {key}")))
}

pub fn require_u64(args: &Value, key: &str) -> Result<u64> {
    if let Some(n) = args.get(key).and_then(|v| v.as_u64()) {
        return Ok(n);
    }
    if let Some(s) = args.get(key).and_then(|v| v.as_str()) {
        if let Ok(n) = s.parse::<u64>() {
            return Ok(n);
        }
    }
    Err(CoworkerError::Workflow(format!("missing or invalid {key}")))
}

pub fn optional_u32(args: &Value, key: &str, default: u32) -> u32 {
    args.get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .or_else(|| {
            args.get(key)
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
        })
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

pub fn require_i64(args: &Value, key: &str) -> Result<i64> {
    if let Some(n) = args.get(key).and_then(|v| v.as_i64()) {
        return Ok(n);
    }
    if let Some(n) = args.get(key).and_then(|v| v.as_u64()) {
        return Ok(n as i64);
    }
    if let Some(s) = args.get(key).and_then(|v| v.as_str()) {
        if let Ok(n) = s.parse::<i64>() {
            return Ok(n);
        }
    }
    Err(CoworkerError::Workflow(format!("missing or invalid {key}")))
}

pub fn optional_i64(args: &Value, key: &str, default: i64) -> i64 {
    args.get(key)
        .and_then(|v| v.as_i64())
        .or_else(|| args.get(key).and_then(|v| v.as_u64()).map(|n| n as i64))
        .or_else(|| {
            args.get(key)
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(default)
}

pub fn optional_bool(args: &Value, key: &str, default: bool) -> bool {
    match args.get(key) {
        None => default,
        Some(v) if v.is_null() => default,
        Some(v) if let Some(b) = v.as_bool() => b,
        Some(v) if let Some(s) = v.as_str() => !s.eq_ignore_ascii_case("false") && s != "0",
        _ => default,
    }
}
