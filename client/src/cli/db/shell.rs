//! `libllm db shell` — interactive SQL REPL.
//!
//! Uses rustyline for line editing, history, and Ctrl+R reverse search on a
//! TTY. On non-TTY stdin, rustyline transparently falls back to a line-buffered
//! reader, which is what lets integration tests script the REPL.
//!
//! Bash's `HISTCONTROL=ignorespace` semantics: a statement whose first input
//! line begins with whitespace is excluded from both on-disk and in-memory
//! history.

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use libllm::db::{Database, QueryRows};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

use super::format::{Format, RowFormatter};
use super::parser::is_statement_complete;
use super::DbContext;

const PROMPT_PRIMARY: &str = "libllm> ";
const PROMPT_CONTINUE: &str = "   ...> ";

struct ShellState {
    db: Database,
    write_allowed: bool,
    format: Format,
    show_headers: bool,
    timer: bool,
    history_path: Option<PathBuf>,
}

impl ShellState {
    fn formatter(&self) -> Box<dyn RowFormatter> {
        self.format.formatter()
    }
}

enum DotCommandOutcome {
    Continue,
    Quit,
}

pub fn run(ctx: &DbContext, write: bool, private: bool) -> Result<()> {
    let db = Database::open(&ctx.db_path, ctx.key.as_ref())?;
    db.execute_batch("PRAGMA query_only = ON;")
        .context("failed to engage query_only mode")?;

    let mut state = ShellState {
        db,
        write_allowed: write,
        format: Format::Table,
        show_headers: true,
        timer: false,
        history_path: if private { None } else { Some(ctx.data_dir.join(".db_shell_history")) },
    };

    let mut editor = DefaultEditor::new().context("failed to create line editor")?;
    if let Some(path) = state.history_path.as_ref()
        && path.exists()
    {
        let _ = editor.load_history(path);
    }

    let mut buffer = String::new();
    let mut buffer_first_line_starts_with_space = false;

    loop {
        let prompt = if buffer.is_empty() {
            PROMPT_PRIMARY
        } else {
            PROMPT_CONTINUE
        };
        match editor.readline(prompt) {
            Ok(line) => {
                if buffer.is_empty() {
                    buffer_first_line_starts_with_space = line
                        .chars()
                        .next()
                        .is_some_and(|c| c == ' ' || c == '\t');
                    let trimmed = line.trim_start();
                    if trimmed.starts_with('.') {
                        match handle_dot_command(&mut state, trimmed) {
                            Ok(DotCommandOutcome::Quit) => break,
                            Ok(DotCommandOutcome::Continue) => {}
                            Err(err) => eprintln!("{err:#}"),
                        }
                        continue;
                    }
                }
                if !buffer.is_empty() {
                    buffer.push('\n');
                }
                buffer.push_str(&line);
                if is_statement_complete(&buffer) {
                    let stmt = std::mem::take(&mut buffer);
                    if !buffer_first_line_starts_with_space {
                        let _ = editor.add_history_entry(stmt.trim_end());
                    }
                    if let Err(err) = run_statement(&state, &stmt) {
                        eprintln!("{err:#}");
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                buffer.clear();
                buffer_first_line_starts_with_space = false;
                eprintln!("(interrupted)");
            }
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                eprintln!("readline error: {err:#}");
                break;
            }
        }
    }

    if let Some(path) = state.history_path.as_ref() {
        let _ = editor.save_history(path);
    }
    Ok(())
}

fn handle_dot_command(state: &mut ShellState, line: &str) -> Result<DotCommandOutcome> {
    let mut parts = line.splitn(2, |c: char| c.is_whitespace());
    let cmd = parts.next().unwrap_or(".help");
    let arg = parts.next().map(str::trim);

    match cmd {
        ".help" => {
            print_help();
            Ok(DotCommandOutcome::Continue)
        }
        ".quit" | ".exit" => Ok(DotCommandOutcome::Quit),
        ".tables" => {
            let rows = state.db.execute_query(
                "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
            )?;
            print_rows(state, &rows);
            Ok(DotCommandOutcome::Continue)
        }
        ".schema" => {
            let rows = match arg {
                Some(name) if !name.is_empty() => {
                    let sql = format!(
                        "SELECT sql FROM sqlite_master WHERE type IN ('table','index') AND name = '{}'",
                        name.replace('\'', "''")
                    );
                    state.db.execute_query(&sql)?
                }
                _ => state.db.execute_query(
                    "SELECT sql FROM sqlite_master WHERE type IN ('table','index')",
                )?,
            };
            print_rows(state, &rows);
            Ok(DotCommandOutcome::Continue)
        }
        ".read" => {
            let path = arg.context(".read requires a file path")?;
            let contents = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read {path}"))?;
            let mut buffer = String::new();
            for raw_line in contents.lines() {
                if !buffer.is_empty() {
                    buffer.push('\n');
                }
                buffer.push_str(raw_line);
                if is_statement_complete(&buffer) {
                    let stmt = std::mem::take(&mut buffer);
                    run_statement(state, &stmt)?;
                }
            }
            Ok(DotCommandOutcome::Continue)
        }
        ".write" => {
            let value = arg.unwrap_or("");
            match value {
                "on" => {
                    if !state.write_allowed {
                        anyhow::bail!(
                            "shell launched without --write; restart with --write to enable mutations"
                        );
                    }
                    state.db.execute_batch("PRAGMA query_only = OFF;")?;
                    Ok(DotCommandOutcome::Continue)
                }
                "off" => {
                    state.db.execute_batch("PRAGMA query_only = ON;")?;
                    Ok(DotCommandOutcome::Continue)
                }
                _ => anyhow::bail!(".write expects 'on' or 'off'"),
            }
        }
        ".mode" => {
            let value = arg.context(".mode requires a format name")?;
            let format = Format::parse(value)
                .with_context(|| format!("unknown mode: {value} (expected: table, pipe, csv, json)"))?;
            state.format = format;
            Ok(DotCommandOutcome::Continue)
        }
        ".headers" => {
            let value = arg.context(".headers expects 'on' or 'off'")?;
            state.show_headers = match value {
                "on" => true,
                "off" => false,
                _ => anyhow::bail!(".headers expects 'on' or 'off'"),
            };
            Ok(DotCommandOutcome::Continue)
        }
        ".timer" => {
            let value = arg.context(".timer expects 'on' or 'off'")?;
            state.timer = match value {
                "on" => true,
                "off" => false,
                _ => anyhow::bail!(".timer expects 'on' or 'off'"),
            };
            Ok(DotCommandOutcome::Continue)
        }
        other => anyhow::bail!("unknown dot-command: {other}"),
    }
}

fn print_help() {
    eprintln!(
        ".help                show this message
.quit, .exit         exit the shell
.tables              list tables
.schema [name]       show table/index DDL
.read <file>         execute SQL from a file
.write on|off        toggle mutation gate (requires --write)
.mode <format>       table | pipe | csv | json
.headers on|off      toggle column headers in output
.timer on|off        print elapsed wall time per statement"
    );
}

fn run_statement(state: &ShellState, sql: &str) -> Result<()> {
    let started = std::time::Instant::now();
    let rows = state.db.execute_query(sql)?;
    if rows.headers.is_empty() {
        let affected = state.db.changes();
        eprintln!("{affected} row(s) affected");
    } else {
        print_rows(state, &rows);
    }
    if state.timer {
        let elapsed = started.elapsed();
        eprintln!("({elapsed:?})");
    }
    Ok(())
}

fn print_rows(state: &ShellState, rows: &QueryRows) {
    let formatter = state.formatter();
    let output = formatter.format(&rows.headers, &rows.rows, state.show_headers);
    let _ = io::stdout().write_all(output.as_bytes());
}
