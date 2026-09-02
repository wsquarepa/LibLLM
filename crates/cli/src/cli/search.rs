//! `libllm search <query>` — full-text search across every stored message.

use std::io::{self, IsTerminal, Write};

use anyhow::{Context, Result};
use libllm_core::crypto;
use libllm_storage::db::Database;
use libllm_storage::search::{self, query as search_query, strip_terminal_controls};
use time::format_description::well_known::Rfc3339;

use crate::cli::Args;

const HIGHLIGHT_OPEN: char = '\u{1}';
const HIGHLIGHT_CLOSE: char = '\u{2}';

pub fn dispatch(args: &Args, query: &str, limit: usize, json: bool, full: bool) -> Result<()> {
    libllm_storage::db::suppress_sqlcipher_log();
    let db = open_db(args)?;
    let compiled = match search_query::compile(query, &db) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(2);
        }
    };
    let hits = search::search(&db, &compiled, limit).context("search execution failed")?;

    if json {
        write_json(&hits)?;
    } else {
        write_text(&hits, full)?;
    }
    Ok(())
}

fn open_db(args: &Args) -> Result<Database> {
    let data_dir = args.data.clone().unwrap_or_else(libllm_config::data_dir);
    let db_path = data_dir.join("data.db");

    let key = if args.no_encrypt {
        None
    } else {
        let passkey = match args.passkey.clone() {
            Some(pk) => pk,
            None => {
                eprint!("Passkey: ");
                rpassword::read_password().context("failed to read interactive passkey")?
            }
        };
        let salt_path = data_dir.join(".salt");
        let salt = crypto::load_or_create_salt(&salt_path)?;
        Some(crypto::derive_key(&passkey, &salt)?)
    };

    let db = Database::open(&db_path, key.as_ref())?;
    db.execute_batch("PRAGMA query_only = ON;")
        .context("failed to engage query_only mode")?;
    Ok(db)
}

#[expect(
    clippy::expect_used,
    reason = "message timestamps were written by this app and are within the RFC 3339 range"
)]
fn write_text(hits: &[search::SearchHit], full: bool) -> Result<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let tty = handle.is_terminal();

    for hit in hits {
        let name = strip_terminal_controls(&hit.session_display_name);
        writeln!(
            handle,
            "[{} | {} | {}]",
            name,
            hit.role,
            hit.timestamp.format(&Rfc3339).expect("RFC 3339 format")
        )?;
        let body = if full {
            render_hl(&hit.preview_text, tty)
        } else {
            render_hl(&hit.snippet, tty)
        };
        writeln!(handle, "  {body}")?;
        writeln!(handle)?;
    }
    writeln!(handle, "{} hits.", hits.len())?;
    Ok(())
}

fn render_hl(input: &str, tty: bool) -> String {
    let sanitized = strip_terminal_controls(input);
    let mut out = String::with_capacity(sanitized.len());
    for c in sanitized.chars() {
        match c {
            HIGHLIGHT_OPEN => {
                if tty {
                    out.push_str("\x1b[1m");
                } else {
                    out.push_str("**");
                }
            }
            HIGHLIGHT_CLOSE => {
                if tty {
                    out.push_str("\x1b[0m");
                } else {
                    out.push_str("**");
                }
            }
            other => out.push(other),
        }
    }
    out
}

#[expect(
    clippy::expect_used,
    reason = "message timestamps were written by this app and are within the RFC 3339 range"
)]
fn write_json(hits: &[search::SearchHit]) -> Result<()> {
    let entries: Vec<serde_json::Value> = hits
        .iter()
        .map(|h| {
            serde_json::json!({
                "session_id": h.session_id,
                "session_display_name": h.session_display_name,
                "message_id": h.message_id,
                "role": h.role.to_string(),
                "timestamp": h.timestamp.format(&Rfc3339).expect("RFC 3339 format"),
                "snippet": h.snippet,
                "preview_text": h.preview_text,
                "score": h.score,
            })
        })
        .collect();
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    serde_json::to_writer(&mut handle, &entries).context("failed to write JSON")?;
    handle.write_all(b"\n").context("failed to flush newline")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::render_hl;

    #[test]
    fn render_hl_strips_csi_escape_sequences() {
        assert_eq!(render_hl("\x1b[1mhello\x1b[0m", false), "hello");
    }

    #[test]
    fn render_hl_strips_csi_and_preserves_fts_markers_non_tty() {
        // U+0001 and U+0002 must survive strip and be rendered as ** markers
        assert_eq!(render_hl("\x1b[1m\u{1}hi\u{2}\x1b[0m", false), "**hi**");
    }

    #[test]
    fn render_hl_strips_csi_and_preserves_fts_markers_tty() {
        // Attacker injects \x1b[1m before the FTS open-marker; after sanitization the
        // injected sequence is gone and the renderer emits its own bold code exactly once.
        let result = render_hl("\x1b[1m\u{1}hi\u{2}\x1b[0m", true);
        assert_eq!(
            result, "\x1b[1mhi\x1b[0m",
            "attacker-supplied sequences must not appear; renderer codes must appear once"
        );
    }

    #[test]
    fn render_hl_strips_osc52_from_input() {
        let result = render_hl("before\x1b]52;c;U0VDUkVU\x07after", false);
        assert_eq!(result, "beforeafter");
    }
}
