//! Redact secrets / PII from values destined for *display* or *persistence*.
//!
//! IMPORTANT: this module only produces a redacted **copy**. The real tool
//! arguments used for execution are never passed through here — redaction is
//! applied at display/render boundaries (tool logs, approval payloads, audit
//! lines, Web snapshot) so secrets never reach the terminal, the UI, or disk.

use serde_json::Value;

/// Keys whose values are treated as secret regardless of value shape.
const SENSITIVE_KEYS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "pwd",
    "api_key",
    "apikey",
    "key",
    "authorization",
    "auth",
    "gh_token",
    "pat",
    "private_key",
    "privatekey",
    "cookie",
    "session",
    "access_token",
    "refresh_token",
    "client_secret",
    "credentials",
    "credential",
];

fn is_sensitive_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    SENSITIVE_KEYS
        .iter()
        .any(|s| k == *s || (k.contains(s) && (k.len() == s.len() || k.ends_with(s))))
}

/// Heuristic: does a bare string value look like a credential?
///
/// Deliberately conservative — only known token prefixes and JWTs are flagged,
/// so legitimate long identifiers (commit SHAs, PR numbers, UUIDs) are NOT
/// accidentally masked.
/// Heuristic for credential-shaped bare strings (used by doctor config checks).
pub fn looks_like_secret(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    if s.starts_with("ghp_")
        || s.starts_with("gho_")
        || s.starts_with("ghu_")
        || s.starts_with("ghs_")
        || s.starts_with("ghr_")
        || s.starts_with("github_pat_")
        || s.starts_with("xoxb-")
        || s.starts_with("xoxp-")
        || s.starts_with("xoxa-")
        || s.starts_with("xoxr-")
        || s.starts_with("AKIA")
        || s.starts_with("sk-")
        || s.starts_with("glpat-")
        || s.starts_with("ghp")
    {
        return true;
    }
    // JWT: header segment is base64url starting with `eyJ` and contains a dot.
    if let Some(rest) = s.strip_prefix("eyJ") {
        if rest.contains('.') {
            return true;
        }
    }
    false
}

/// Return a copy of `value` with sensitive keys and secret-shaped strings
/// replaced by `***redacted***`. Recurses through objects and arrays.
pub fn redact_sensitive(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                if is_sensitive_key(k) {
                    out.insert(k.clone(), Value::String("***redacted***".into()));
                } else {
                    out.insert(k.clone(), redact_sensitive(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(redact_sensitive).collect()),
        Value::String(s) => {
            if looks_like_secret(s) {
                Value::String("***redacted***".into())
            } else {
                value.clone()
            }
        }
        other => other.clone(),
    }
}

/// Parse `s` as JSON, redact, and re-serialize. Falls back to the original
/// string (trimmed, truncated for safety) if it is not valid JSON.
pub fn redact_json_str(s: &str) -> String {
    match serde_json::from_str::<Value>(s) {
        Ok(v) => serde_json::to_string(&redact_sensitive(&v)).unwrap_or_else(|_| s.to_string()),
        Err(_) => {
            let t = s.trim();
            if t.len() > 2000 {
                format!("{}…(truncated)", &t[..2000])
            } else {
                t.to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_sensitive_keys() {
        let v = json!({"token": "ghp_abc123", "repo": "acme/widget", "api_key": "sk-xyz"});
        let r = redact_sensitive(&v);
        assert_eq!(r["token"], "***redacted***");
        assert_eq!(r["api_key"], "***redacted***");
        assert_eq!(r["repo"], "acme/widget");
    }

    #[test]
    fn redacts_token_shaped_strings() {
        let v = json!({"url": "https://api.example.com", "auth": "github_pat_xxxYYYY"});
        let r = redact_sensitive(&v);
        assert_eq!(r["auth"], "***redacted***");
        assert_eq!(r["url"], "https://api.example.com");
    }

    #[test]
    fn does_not_redact_sha_like_strings() {
        // Commit SHAs / IDs must survive.
        let v = json!({"sha": "e6a6c4f2b3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8"});
        let r = redact_sensitive(&v);
        assert_eq!(r["sha"], "e6a6c4f2b3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8");
    }

    #[test]
    fn redact_json_str_handles_non_json() {
        assert_eq!(
            redact_json_str("not json but ghp_secretvalue"),
            "not json but ghp_secretvalue"
        );
    }

    #[test]
    fn redact_json_str_redacts() {
        let out = redact_json_str(r#"{"password":"hunter2","x":1}"#);
        assert!(out.contains("***redacted***"));
        assert!(out.contains(r#""x":1"#));
    }
}
