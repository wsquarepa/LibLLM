//! File-ingestion pipeline: tokenises `@<path>` references out of a chat
//! message, resolves and classifies them, and produces the snapshot
//! `Role::System` messages that get pushed alongside the user message.
//!
//! See the file-ingestion design spec for the full contract.

mod error;

pub use error::{DelimiterKind, FileError};
