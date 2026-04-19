//! `libllm db import <path>` — replace the encrypted database with the
//! contents of a plaintext SQLite file at <path>. Always backs up first;
//! aborts on schema-version mismatch.

use std::path::Path;

use anyhow::{Context, Result};
use libllm::config::BackupConfig;

use super::exit;
use super::{DbContext, confirm_yes, wal_liveness_check};

pub fn run(ctx: &DbContext, yes: bool, path: &Path) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("plaintext file does not exist: {}", path.display());
    }

    let plain_version = read_schema_version(path)?;
    let expected = libllm::db::CURRENT_VERSION;
    if plain_version != expected {
        eprintln!(
            "plaintext schema version {plain_version} does not match \
             binary schema version {expected}; aborting import to avoid corruption"
        );
        std::process::exit(exit::SCHEMA_MISMATCH);
    }

    if !yes {
        let prompt = format!(
            "Replace contents of {} with {}?",
            ctx.db_path.display(),
            path.display()
        );
        if !confirm_yes(&prompt)? {
            std::process::exit(exit::USER_DECLINED);
        }
    }

    if let Err(err) = wal_liveness_check(&ctx.db_path, ctx.key.as_ref()) {
        eprintln!("{err:#}");
        std::process::exit(exit::WAL_LIVENESS);
    }

    if ctx.db_path.exists() {
        let backup_config = BackupConfig {
            enabled: true,
            ..BackupConfig::default()
        };
        backup::snapshot::create_snapshot(&ctx.data_dir, ctx.passkey.as_deref(), &backup_config)
            .context("mandatory pre-import backup failed; refusing to proceed")?;
    }

    let tmp_path = ctx.db_path.with_extension("import.tmp");
    if tmp_path.exists() {
        std::fs::remove_file(&tmp_path)
            .with_context(|| format!("failed to remove stale tmp file: {}", tmp_path.display()))?;
    }

    let build_result = build_replacement(&tmp_path, path, ctx.key.as_ref());
    if let Err(err) = build_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }

    std::fs::rename(&tmp_path, &ctx.db_path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            ctx.db_path.display()
        )
    })?;

    let wal = ctx.db_path.with_extension("db-wal");
    let shm = ctx.db_path.with_extension("db-shm");
    for sidecar in [&wal, &shm] {
        if sidecar.exists()
            && let Err(err) = std::fs::remove_file(sidecar)
        {
            eprintln!(
                "warning: failed to remove stale sidecar {}: {err}",
                sidecar.display()
            );
        }
    }

    eprintln!("Imported {} into {}", path.display(), ctx.db_path.display());
    Ok(())
}

fn read_schema_version(path: &Path) -> Result<i64> {
    let conn = rusqlite::Connection::open(path)
        .with_context(|| format!("failed to open plaintext file: {}", path.display()))?;
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get::<_, i64>(0),
    )
    .with_context(|| format!("failed to read schema_version from {}", path.display()))
}

fn build_replacement(
    tmp_path: &Path,
    plain_path: &Path,
    key: Option<&libllm::crypto::DerivedKey>,
) -> Result<()> {
    let conn = rusqlite::Connection::open(tmp_path)
        .with_context(|| format!("failed to open tmp db: {}", tmp_path.display()))?;
    if let Some(key) = key {
        conn.execute_batch(&key.key_pragma())
            .context("failed to set encryption key on tmp db")?;
    }
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
        .context("failed to set pragmas on tmp db")?;
    let plain_str = plain_path.to_str().context(
        "plaintext path contains non-UTF-8 bytes; SQLCipher ATTACH requires a valid string path",
    )?;
    let script = format!(
        "ATTACH DATABASE '{}' AS plain KEY '';\n\
         SELECT sqlcipher_export('main', 'plain');\n\
         DETACH DATABASE plain;",
        plain_str.replace('\'', "''")
    );
    conn.execute_batch(&script)
        .context("failed to copy plaintext into tmp db")?;
    Ok(())
}
