//! `libllm db import <path>` — replace the encrypted database with the contents
//! of a plaintext SQLite file at <path>.

use std::path::Path;

use anyhow::Result;

use super::DbContext;

pub fn run(_ctx: &DbContext, _yes: bool, _path: &Path) -> Result<()> {
    anyhow::bail!("db import is not yet implemented")
}
