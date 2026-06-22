use crate::error::CoworkerError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrCode {
    Auth,
    NotFound,
    Forbidden,
    RateLimit,
    Transient,
    Validation,
    ExternalCi,
    Unavailable,
    Generic,
}

impl ErrCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "AUTH",
            Self::NotFound => "NOT_FOUND",
            Self::Forbidden => "FORBIDDEN",
            Self::RateLimit => "RATE_LIMIT",
            Self::Transient => "TRANSIENT",
            Self::Validation => "VALIDATION",
            Self::ExternalCi => "EXTERNAL_CI",
            Self::Unavailable => "UNAVAILABLE",
            Self::Generic => "GENERIC",
        }
    }
}

pub fn format_tool_error(code: ErrCode, message: &str, hint: &str) -> String {
    crate::agent::harness_errors::format_error_line(code.as_str(), message, hint)
}

pub fn format_tool_ok(summary: &str) -> String {
    format!("OK: {summary}")
}

pub fn not_implemented_yet(tool: &str) -> CoworkerError {
    CoworkerError::Other(anyhow::anyhow!(format_tool_error(
        ErrCode::Unavailable,
        &format!("tool {tool} is not implemented in harness yet"),
        "Upgrade unistar-coworker or use a workflow that does not require this tool",
    )))
}
