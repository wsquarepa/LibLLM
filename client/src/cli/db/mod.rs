//! `libllm db` subcommand group: direct inspection and editing of the encrypted
//! database via the existing decryption pipeline.

pub mod dump;
pub mod format;
pub mod import;
pub mod parser;
pub mod shell;
pub mod sql;

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use libllm::crypto::{self, DerivedKey};

use crate::cli::{Args, DbSubcommand};

/// Resolved context shared by all four db subcommands.
pub struct DbContext {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub passkey: Option<String>,
    pub key: Option<DerivedKey>,
}

pub fn dispatch(args: &Args, command: &DbSubcommand) -> Result<()> {
    let ctx = resolve_context(args)?;
    match command {
        DbSubcommand::Sql {
            write,
            format,
            query,
        } => sql::run(&ctx, *write, format, query),
        DbSubcommand::Shell { write, private } => shell::run(&ctx, *write, *private),
        DbSubcommand::Dump { yes, path } => dump::run(&ctx, *yes, path),
        DbSubcommand::Import { yes, path } => import::run(&ctx, *yes, path),
    }
}

fn resolve_context(args: &Args) -> Result<DbContext> {
    let data_dir = args
        .data
        .clone()
        .unwrap_or_else(libllm::config::data_dir);
    let db_path = data_dir.join("data.db");

    if args.no_encrypt {
        return Ok(DbContext {
            data_dir,
            db_path,
            passkey: None,
            key: None,
        });
    }

    let passkey = match args.passkey.clone() {
        Some(pk) => pk,
        None => {
            eprint!("Passkey: ");
            rpassword::read_password().context("failed to read interactive passkey")?
        }
    };

    let salt_path = data_dir.join(".salt");
    let salt = crypto::load_or_create_salt(&salt_path)?;
    let key = crypto::derive_key(&passkey, &salt)?;

    Ok(DbContext {
        data_dir,
        db_path,
        passkey: Some(passkey),
        key: Some(key),
    })
}

/// Probe whether another process holds the database. We attempt to acquire an
/// immediate write lock; on WAL-mode SQLite this fails with `SQLITE_BUSY` when
/// another connection has a pending write.
pub fn wal_liveness_check(db_path: &Path, key: Option<&DerivedKey>) -> Result<()> {
    let conn = rusqlite::Connection::open(db_path)
        .with_context(|| format!("failed to open database for liveness check: {}", db_path.display()))?;
    if let Some(key) = key {
        conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", key.hex()))
            .context("failed to set encryption key for liveness check")?;
    }
    conn.busy_timeout(std::time::Duration::from_millis(0))
        .context("failed to set busy_timeout for liveness check")?;
    let probe = conn.execute_batch("BEGIN IMMEDIATE; ROLLBACK;");
    if let Err(rusqlite::Error::SqliteFailure(err, _)) = &probe
        && err.code == rusqlite::ErrorCode::DatabaseBusy
    {
        anyhow::bail!(
            "another LibLLM process appears to be using the database; \
             close it before running this db subcommand"
        );
    }
    probe.context("liveness check failed")?;
    Ok(())
}

/// Prompt the user for `y/N` confirmation. Returns true on `y`/`Y`/`yes`,
/// false otherwise (including EOF and empty input).
pub fn confirm_yes(message: &str) -> Result<bool> {
    eprint!("{message} [y/N] ");
    io::stderr().flush().ok();
    let mut line = String::new();
    let read = io::stdin().lock().read_line(&mut line)?;
    if read == 0 {
        return Ok(false);
    }
    let trimmed = line.trim();
    Ok(trimmed.eq_ignore_ascii_case("y") || trimmed.eq_ignore_ascii_case("yes"))
}

/// Standard exit codes shared across db subcommands.
pub mod exit {
    pub const USER_DECLINED: i32 = 2;
    pub const SCHEMA_MISMATCH: i32 = 3;
    pub const WAL_LIVENESS: i32 = 4;
}
