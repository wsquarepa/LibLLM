//! Typed error type for the protocol layer.

use crate::client::AuthError;

/// All errors produced by the protocol layer (HTTP client, tokenizer, summarizer).
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// A reqwest transport, connection, or response-parse error.
    ///
    /// Stored as already-redacted display text so secrets in request URLs
    /// (query values, userinfo) never surface via `Display`.
    #[error("{0}")]
    Request(String),

    /// Authentication configuration could not be applied to the request.
    #[error(transparent)]
    Auth(#[from] AuthError),

    /// The server returned a non-2xx HTTP status.
    #[error("API returned {status}: {body}")]
    HttpStatus { status: u16, body: String },

    /// A required field was absent from a successful server response.
    #[error("{endpoint} response missing `{field}`")]
    MissingField {
        endpoint: &'static str,
        field: &'static str,
    },

    /// An I/O error writing to the output stream during streaming completion.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// A server probe timed out (used during tokenizer backend selection).
    #[error("{0}")]
    Timeout(String),
}

impl From<reqwest::Error> for ApiError {
    fn from(err: reqwest::Error) -> Self {
        ApiError::Request(crate::redact::redact_error_text(&err.to_string()))
    }
}

/// Convenience alias used throughout the protocol crate.
pub type Result<T> = std::result::Result<T, ApiError>;
