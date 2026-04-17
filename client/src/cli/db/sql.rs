//! `libllm db sql <query>` — one-shot SQL runner.

use anyhow::Result;

use super::DbContext;

pub fn run(_ctx: &DbContext, _write: bool, _format: &str, _query: &str) -> Result<()> {
    anyhow::bail!("db sql is not yet implemented")
}
