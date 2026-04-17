//! `libllm db dump <path>` — write a decrypted SQLite database to <path>.

use std::path::Path;

use anyhow::{Context, Result};
use libllm::db::Database;

use super::exit;
use super::{confirm_yes, wal_liveness_check, DbContext};

pub fn run(ctx: &DbContext, yes: bool, path: &Path) -> Result<()> {
    if path.exists() && !yes {
        let prompt = format!("Overwrite {}?", path.display());
        if !confirm_yes(&prompt)? {
            std::process::exit(exit::USER_DECLINED);
        }
    }

    if let Err(err) = wal_liveness_check(&ctx.db_path, ctx.key.as_ref()) {
        eprintln!("{err:#}");
        std::process::exit(exit::WAL_LIVENESS);
    }

    let tmp_path = path.with_extension("tmp");
    if tmp_path.exists() {
        std::fs::remove_file(&tmp_path)
            .with_context(|| format!("failed to remove stale tmp file: {}", tmp_path.display()))?;
    }

    let result = (|| -> Result<()> {
        let db = Database::open(&ctx.db_path, ctx.key.as_ref())?;
        let script = format!(
            "ATTACH DATABASE '{}' AS plain KEY '';\n\
             SELECT sqlcipher_export('plain');\n\
             DETACH DATABASE plain;",
            tmp_path.display().to_string().replace('\'', "''")
        );
        db.execute_batch(&script)?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            std::fs::rename(&tmp_path, path).with_context(|| {
                format!(
                    "failed to rename {} to {}",
                    tmp_path.display(),
                    path.display()
                )
            })?;
            eprintln!("Wrote decrypted database to {}", path.display());
            Ok(())
        }
        Err(err) => {
            let _ = std::fs::remove_file(&tmp_path);
            Err(err)
        }
    }
}
