//! `libllm db dump <path>` — write a decrypted SQLite database to <path>.

use std::path::Path;

use anyhow::Result;

use super::DbContext;

pub fn run(_ctx: &DbContext, _yes: bool, _path: &Path) -> Result<()> {
    anyhow::bail!("db dump is not yet implemented")
}
