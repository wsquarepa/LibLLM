//! Interactive TTY helpers over `dialoguer`.
//!
//! Wraps `Select` and `Confirm` with a consistent cancellation model
//! (Esc / Ctrl+C return `Ok(None)`) and centralizes TTY detection so
//! each subcommand does not re-implement it.

use std::io::{self, IsTerminal};

use anyhow::{Context, Result};
use dialoguer::{Confirm, Select, theme::ColorfulTheme};

/// Returns true when both stdin and stderr are TTYs.
///
/// Dialoguer writes prompts to stderr and reads from stdin; both must
/// be terminals for arrow-key selection to function.
pub fn is_interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

/// Show an arrow-key selector and return the chosen index.
///
/// Returns `Ok(None)` when the user cancels with Esc or Ctrl+C.
/// Returns `Err` only on I/O failures writing to the terminal.
pub fn select<T: ToString>(prompt: &str, items: &[T]) -> Result<Option<usize>> {
    Select::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .items(items)
        .default(0)
        .interact_opt()
        .context("failed to show selection prompt")
}

/// Show a yes/no confirm prompt.
///
/// Returns `Ok(None)` when the user cancels with Esc or Ctrl+C.
pub fn confirm(prompt: &str, default: bool) -> Result<Option<bool>> {
    Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .default(default)
        .interact_opt()
        .context("failed to show confirm prompt")
}
