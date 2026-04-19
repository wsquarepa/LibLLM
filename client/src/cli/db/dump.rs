//! `libllm db dump <path>` — write a decrypted SQLite database to <path>.

use std::path::Path;

use anyhow::{Context, Result};
use libllm::crypto::chmod_0600;
use libllm::db::Database;

use super::exit;
use super::{DbContext, confirm_yes, wal_liveness_check};

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

    let tmp_path = {
        let mut s = path.as_os_str().to_owned();
        s.push(".tmp");
        std::path::PathBuf::from(s)
    };
    if tmp_path.exists() {
        std::fs::remove_file(&tmp_path)
            .with_context(|| format!("failed to remove stale tmp file: {}", tmp_path.display()))?;
    }

    let result = (|| -> Result<()> {
        let db = Database::open(&ctx.db_path, ctx.key.as_ref())?;
        let tmp_str = tmp_path.to_str().context(
            "tmp path contains non-UTF-8 bytes; SQLCipher ATTACH requires a valid string path",
        )?;
        let script = format!(
            "ATTACH DATABASE '{}' AS plain KEY '';\n\
             SELECT sqlcipher_export('plain');\n\
             DETACH DATABASE plain;",
            tmp_str.replace('\'', "''")
        );
        db.execute_batch(&script)?;
        chmod_0600(&tmp_path)
            .with_context(|| format!("failed to restrict permissions: {}", tmp_path.display()))?;
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
            chmod_0600(path)
                .with_context(|| format!("failed to restrict permissions: {}", path.display()))?;
            eprintln!("Wrote decrypted database to {}", path.display());
            Ok(())
        }
        Err(err) => {
            let _ = std::fs::remove_file(&tmp_path);
            Err(err)
        }
    }
}
