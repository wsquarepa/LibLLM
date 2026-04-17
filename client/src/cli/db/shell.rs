//! `libllm db shell` — interactive SQL REPL.

use anyhow::Result;

use super::DbContext;

pub fn run(_ctx: &DbContext, _write: bool, _private: bool) -> Result<()> {
    anyhow::bail!("db shell is not yet implemented")
}
