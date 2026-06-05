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
mod summary;

pub use classify::{Classified, classify};
pub use error::{DelimiterKind, FileError};
pub use parse::{FileReference, file_reference_ranges, unescape_at};
pub use resolve::{
    ResolvedFile, assemble_snapshot_messages, resolve_all, resolve_all_resolved,
    resolve_with_prepended, resolve_with_prepended_resolved, stdin_attachment,
};
pub use rewrite::rewrite_user_message;
pub use snapshot::{
    build_snapshot_body, check_delimiter_collision, is_snapshot, snapshot_basename,
    snapshot_inner_text,
};
pub use summary::{
    FileSummarizer, FileSummary, FileSummaryLookup, FileSummaryStatus, FileToSummarize,
    NullFileSummaryLookup, ReadyEvent, ScopedFileSummaryLookup, SessionScopedLookup,
    check_file_fits,
};

/// Content-addressed hash of `bytes`, rendered as lowercase hex.
///
/// Backed by xxh3-128; not a cryptographic primitive. Used as a dedup key
/// for session-scoped SQLite rows in the `file_summaries` table.
pub fn content_hash_hex(bytes: &[u8]) -> String {
    format!("{:032x}", xxh3_128(bytes))
}

/// Extract `FileToSummarize` inputs from a slice of freshly-resolved
/// messages (the `Vec<Message>` returned by `resolve_all` / similar).
/// Non-snapshot messages are skipped.
pub fn files_to_summarize_from_messages(
    messages: &[crate::session::Message],
) -> Vec<summary::FileToSummarize> {
    messages
        .iter()
        .filter(|m| m.role == crate::session::Role::System)
        .filter_map(|m| {
            let basename = snapshot::snapshot_basename(&m.content)?;
            let inner = snapshot::snapshot_inner_text(&m.content).to_owned();
            if inner.is_empty() {
                return None;
            }
            let content_hash = content_hash_hex(inner.as_bytes());
            Some(summary::FileToSummarize {
                basename,
                content_hash,
                body: inner,
            })
        })
        .collect()
}

#[cfg(test)]
mod files_to_summarize_tests {
    use super::*;
    use crate::session::{Message, Role};

    #[test]
    fn extracts_one_per_snapshot_message() {
        let body_a = snapshot::build_snapshot_body("a.md", "alpha");
        let body_b = snapshot::build_snapshot_body("b.md", "beta");
        let msgs = [
            Message::new(Role::User, "hi".to_owned()),
            Message::new(Role::System, body_a.clone()),
            Message::new(Role::System, body_b.clone()),
        ];
        let out = files_to_summarize_from_messages(&msgs);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].basename, "a.md");
        assert_eq!(out[0].body, "alpha");
        assert_eq!(out[1].basename, "b.md");
        assert_eq!(out[1].body, "beta");
    }

    #[test]
    fn skips_freeform_system_messages() {
        let msgs = [Message::new(Role::System, "You are helpful.".to_owned())];
        assert!(files_to_summarize_from_messages(&msgs).is_empty());
    }

    #[test]
    fn skips_empty_snapshot_body() {
        let body = snapshot::build_snapshot_body("empty.md", "");
        let msgs = [Message::new(Role::System, body)];
        assert!(files_to_summarize_from_messages(&msgs).is_empty());
    }
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
