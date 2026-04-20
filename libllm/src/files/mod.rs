//! File-ingestion pipeline: tokenises `@<path>` references out of a chat
//! message, resolves and classifies them, and produces the snapshot
//! `Role::System` messages that get pushed alongside the user message.
//!
//! See the file-ingestion design spec for the full contract.

use xxhash_rust::xxh3::xxh3_128;

mod classify;
mod error;
mod parse;
mod resolve;
mod rewrite;
mod snapshot;

pub use classify::{Classified, classify};
pub use error::{DelimiterKind, FileError};
pub use parse::{FileReference, file_reference_ranges, unescape_at};
pub use resolve::{ResolvedFile, resolve_all, resolve_with_prepended, stdin_attachment};
pub use rewrite::rewrite_user_message;
pub use snapshot::{
    build_snapshot_body, check_delimiter_collision, is_snapshot, snapshot_basename,
    snapshot_inner_text,
};

/// Content-addressed hash of `bytes`, rendered as lowercase hex.
///
/// Backed by xxh3-128; not a cryptographic primitive. Used as a dedup key
/// for session-scoped SQLite rows in the `file_summaries` table.
pub fn content_hash_hex(bytes: &[u8]) -> String {
    format!("{:032x}", xxh3_128(bytes))
}

#[cfg(test)]
mod mod_tests {
    use super::*;

    #[test]
    fn content_hash_hex_is_deterministic() {
        let h1 = content_hash_hex(b"hello world");
        let h2 = content_hash_hex(b"hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn content_hash_hex_differs_for_different_bytes() {
        let h1 = content_hash_hex(b"hello world");
        let h2 = content_hash_hex(b"hello worlD");
        assert_ne!(h1, h2);
    }

    #[test]
    fn content_hash_hex_is_lowercase_hex_32_chars() {
        let h = content_hash_hex(b"anything");
        assert_eq!(h.len(), 32);
        assert!(h.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    }

    #[test]
    fn content_hash_hex_empty_input_is_stable() {
        let h1 = content_hash_hex(b"");
        let h2 = content_hash_hex(b"");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 32);
    }
}
