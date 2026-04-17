//! `libllm db sql <query>` — one-shot SQL runner.

use std::io::{self, Write};

use anyhow::{Context, Result};
use libllm::db::Database;

use super::format::Format;
use super::parser::is_single_statement;
use super::DbContext;

pub fn run(ctx: &DbContext, write: bool, format: &str, query: &str) -> Result<()> {
    if !is_single_statement(query) {
        anyhow::bail!(
            "db sql accepts a single statement; use db shell or .read for multi-statement scripts"
        );
    }

    let format = Format::parse(format)
        .with_context(|| format!("unknown format: {format} (expected: table, pipe, csv, json)"))?;

    let db = Database::open(&ctx.db_path, ctx.key.as_ref())?;
    if !write {
        db.execute_batch("PRAGMA query_only = ON;")
            .context("failed to engage query_only mode")?;
    }

    let rows = db.execute_query(query)?;
    if rows.headers.is_empty() {
        let affected = db.changes();
        eprintln!("{affected} row(s) affected");
    } else {
        let formatter = format.formatter();
        let output = formatter.format(&rows.headers, &rows.rows, true);
        io::stdout()
            .write_all(output.as_bytes())
            .context("failed to write output")?;
    }
    Ok(())
}
