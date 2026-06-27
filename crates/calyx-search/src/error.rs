//! `calyx-search` error type. A per-crate error that wraps the canonical
//! [`CalyxError`] catalog plus two local conditions (I/O, usage) that have no
//! catalog entry — mirrors the wire contract so a caller can map it onto its own
//! surface (CLI envelope, HTTP error envelope) without losing the structured
//! `code`. No silent fallback: every failure is explicit.
use calyx_core::CalyxError;

#[derive(Debug, Clone)]
pub enum SearchError {
    /// A structured catalog error carried verbatim.
    Calyx(CalyxError),
    /// An OS/I/O or codec failure with no catalog entry.
    Io(String),
    /// A caller-misuse failure (bad filter JSON, path outside root).
    Usage(String),
}

/// Internal alias so moved modules keep using `crate::error::{CliError, CliResult}`.
pub use SearchError as CliError;
pub type CliResult<T = ()> = core::result::Result<T, SearchError>;

impl SearchError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }
    pub fn io(message: impl Into<String>) -> Self {
        Self::Io(message.into())
    }

    /// Stable wire code (mirrors the CLI envelope so callers can dispatch).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Calyx(error) => error.code,
            Self::Io(_) => "CALYX_CLI_IO_ERROR",
            Self::Usage(_) => "CALYX_CLI_USAGE_ERROR",
        }
    }

    /// The concrete failure detail.
    pub fn message(&self) -> &str {
        match self {
            Self::Calyx(error) => &error.message,
            Self::Io(message) | Self::Usage(message) => message,
        }
    }
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Calyx(error) => write!(f, "{error}"),
            Self::Io(message) | Self::Usage(message) => write!(f, "{message}"),
        }
    }
}
impl std::error::Error for SearchError {}

impl From<CalyxError> for SearchError {
    fn from(error: CalyxError) -> Self {
        Self::Calyx(error)
    }
}
impl From<std::io::Error> for SearchError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
impl From<serde_json::Error> for SearchError {
    fn from(error: serde_json::Error) -> Self {
        Self::Io(error.to_string())
    }
}
impl From<String> for SearchError {
    fn from(message: String) -> Self {
        Self::Usage(message)
    }
}
impl From<&str> for SearchError {
    fn from(message: &str) -> Self {
        Self::Usage(message.to_string())
    }
}

/// Boundary conversion for callers that speak the `CalyxError` catalog.
impl From<SearchError> for CalyxError {
    fn from(error: SearchError) -> Self {
        match error {
            SearchError::Calyx(catalog) => catalog,
            SearchError::Io(message) | SearchError::Usage(message) => {
                CalyxError::stale_derived(message)
            }
        }
    }
}
