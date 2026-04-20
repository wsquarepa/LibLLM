//! File-ingestion pipeline: tokenises `@<path>` references out of a chat
//! message, resolves and classifies them, and produces the snapshot
//! `Role::System` messages that get pushed alongside the user message.
//!
//! See the file-ingestion design spec for the full contract.

mod error;
mod parse;
mod rewrite;
mod snapshot;

pub use error::{DelimiterKind, FileError};
pub use parse::{FileReference, file_reference_ranges, unescape_at};
pub use rewrite::rewrite_user_message;
pub use snapshot::{
    build_snapshot_body, check_delimiter_collision, is_snapshot, snapshot_basename,
};
