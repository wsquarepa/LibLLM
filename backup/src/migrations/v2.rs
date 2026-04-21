use anyhow::Result;
use std::path::Path;

use crate::index::BackupIndex;

pub(super) fn migrate(
    _index: &mut BackupIndex,
    _backups_dir: &Path,
    _kek: Option<&[u8; 32]>,
) -> Result<()> {
    anyhow::bail!("v2 migration not yet implemented")
}
