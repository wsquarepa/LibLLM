//! `libllm search <query>` — full-text search across every stored message.

use std::io::{self, IsTerminal, Write};

use anyhow::{Context, Result};
use libllm::crypto;
use libllm::db::Database;
use libllm::search::{self, query as search_query};
use time::format_description::well_known::Rfc3339;

use crate::cli::Args;

const HIGHLIGHT_OPEN: char = '\u{1}';
const HIGHLIGHT_CLOSE: char = '\u{2}';

pub fn dispatch(args: &Args, query: &str, limit: usize, json: bool, full: bool) -> Result<()> {
    libllm::db::suppress_sqlcipher_log();
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
    let data_dir = args.data.clone().unwrap_or_else(libllm::config::data_dir);
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

fn write_text(hits: &[search::SearchHit], full: bool) -> Result<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let tty = handle.is_terminal();

    for hit in hits {
        writeln!(
            handle,
            "[{} | {} | {}]",
            hit.session_display_name,
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
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
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

fn write_json(_hits: &[search::SearchHit]) -> Result<()> {
    anyhow::bail!("--json is implemented in the next task")
}
