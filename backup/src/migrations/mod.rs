use anyhow::{Context, Result};
use std::path::Path;

use crate::index::{BackupIndex, SCHEMA_VERSION};

mod v2;

/// Public entry point. Applies every pending migration in order and stamps
/// the final version on the supplied index. Callers are expected to persist
/// the index after a successful return.
pub fn run_migrations(
    index: &mut BackupIndex,
    backups_dir: &Path,
    kek: Option<&[u8; 32]>,
) -> Result<()> {
    while index.version < SCHEMA_VERSION {
        let next = index.version + 1;
        match next {
            2 => v2::migrate(index, backups_dir, kek)
                .context("v1 -> v2 migration failed")?,
            other => anyhow::bail!("no migration registered for version {other}"),
        }
        stamp_version(index, next);
    }
    Ok(())
}

fn stamp_version(index: &mut BackupIndex, version: u32) {
    index.version = version;
}
