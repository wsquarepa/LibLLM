//! Typed error type for the storage layer.

/// All errors produced by the storage layer.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// A SQL operation failed; `context` describes the operation.
    #[error("{context}: {source}")]
    Query {
        context: String,
        #[source]
        source: rusqlite::Error,
    },

    /// A raw rusqlite error with no extra context.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    /// The requested session was not found.
    #[error("session not found: {id}")]
    SessionNotFound { id: String },

    /// The requested character was not found.
    #[error("character not found: {slug}")]
    CharacterNotFound { slug: String },

    /// The requested persona was not found.
    #[error("persona not found: {slug}")]
    PersonaNotFound { slug: String },

    /// The requested system prompt was not found.
    #[error("system prompt not found: {slug}")]
    PromptNotFound { slug: String },

    /// The requested worldbook was not found.
    #[error("worldbook not found: {slug}")]
    WorldbookNotFound { slug: String },

    /// The database is not available for save (e.g. no db handle in current mode).
    #[error("database not available for save")]
    DatabaseNotAvailable,

    /// A serde_json serialization or deserialization error.
    #[error("{context}: {source}")]
    Json {
        context: String,
        #[source]
        source: serde_json::Error,
    },

    /// System time is before the Unix epoch (should never happen on any reasonable system).
    #[error("system time before Unix epoch")]
    SystemTimeBeforeEpoch,

    /// A file I/O operation failed (e.g. chmod, WAL sidecar permissions).
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
}

/// Convenience alias used throughout the storage crate.
pub type Result<T> = std::result::Result<T, DbError>;
