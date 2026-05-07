//! Interactive TTY helpers over `dialoguer`.
//!
//! Wraps `Select` and `Confirm` with a consistent cancellation model
//! (Esc / Ctrl+C return `Ok(None)`) and centralizes TTY detection so
//! each subcommand does not re-implement it.

use std::fmt::Write as _;
use std::io::{self, IsTerminal};

use anyhow::{Context, Result};
use dialoguer::console::{Key, Style, Term, style};
use dialoguer::{Confirm, Select, theme::ColorfulTheme};

const ARROW_VISIBLE: usize = 9;
const ARROW_ABOVE: usize = 4;

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

/// First index shown in the [`arrow_select`] viewport, given the current selection.
///
/// When `n > ARROW_VISIBLE`, the cursor stays at slot 4 while the viewport
/// scrolls, except near the edges of the list where the viewport freezes
/// (against `0` at the top and against `n - ARROW_VISIBLE` at the bottom)
/// and the cursor moves into the edge slots. Otherwise returns `0`.
fn arrow_window_start(sel: usize, n: usize) -> usize {
    if n <= ARROW_VISIBLE {
        return 0;
    }
    let max_start = n - ARROW_VISIBLE;
    sel.saturating_sub(ARROW_ABOVE).min(max_start)
}

/// Selector that always shows up to 9 rows with the cursor centered when
/// possible. Up/Down clamp at the ends instead of wrapping.
///
/// Returns `Ok(None)` on Esc / Ctrl+C / `q`. `default` is clamped into range.
pub fn arrow_select(prompt: &str, items: &[String], default: usize) -> Result<Option<usize>> {
    if items.is_empty() {
        return Ok(None);
    }

    let term = Term::stderr();
    if !term.is_term() {
        anyhow::bail!("arrow_select requires a TTY");
    }

    let n = items.len();
    let mut sel = default.min(n - 1);
    let prompt_q = style("?".to_string()).for_stderr().yellow();
    let prompt_text = Style::new().for_stderr().bold();
    let cursor = style(">".to_string()).for_stderr().cyan().bold();
    let active_item = Style::new().for_stderr().cyan();

    let render = |sel_idx: usize| -> io::Result<()> {
        let window_start = arrow_window_start(sel_idx, n);
        let visible = ARROW_VISIBLE.min(n);
        let cursor_slot = sel_idx - window_start;
        let mut buf = String::new();
        let _ = writeln!(buf, "{} {}", prompt_q, prompt_text.apply_to(prompt));
        for slot in 0..ARROW_VISIBLE {
            if slot < visible {
                let idx = window_start + slot;
                if slot == cursor_slot {
                    let _ = writeln!(buf, "{} {}", cursor, active_item.apply_to(&items[idx]));
                } else {
                    let _ = writeln!(buf, "  {}", items[idx]);
                }
            } else {
                buf.push('\n');
            }
        }
        term.write_str(&buf)
    };

    term.hide_cursor().context("failed to hide cursor")?;
    render(sel).context("failed to render selector")?;

    let outcome = loop {
        match term.read_key().context("failed to read key")? {
            Key::ArrowDown | Key::Tab | Key::Char('j') => sel = (sel + 1).min(n - 1),
            Key::ArrowUp | Key::BackTab | Key::Char('k') => sel = sel.saturating_sub(1),
            Key::Enter => break Some(sel),
            Key::Escape | Key::CtrlC | Key::Char('q') => break None,
            _ => continue,
        }
        term.clear_last_lines(ARROW_VISIBLE + 1)
            .context("failed to clear selector frame")?;
        render(sel).context("failed to render selector")?;
    };

    term.clear_last_lines(ARROW_VISIBLE + 1)
        .context("failed to clear selector frame")?;
    term.show_cursor().context("failed to restore cursor")?;

    if let Some(idx) = outcome {
        let summary_q = style("?".to_string()).for_stderr().green();
        let value = Style::new().for_stderr().green().apply_to(&items[idx]);
        term.write_str(&format!(
            "{} {} {}\n",
            summary_q,
            prompt_text.apply_to(prompt),
            value
        ))
        .context("failed to write selection summary")?;
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_centers_selection_when_list_exceeds_viewport() {
        let start = arrow_window_start(10, 20);
        assert_eq!(start, 6);
        assert_eq!(10 - start, ARROW_ABOVE);
    }

    #[test]
    fn window_freezes_at_top_when_selection_near_start() {
        let n = 20;
        assert_eq!(arrow_window_start(0, n), 0);
        assert_eq!(arrow_window_start(1, n), 0);
        assert_eq!(arrow_window_start(ARROW_ABOVE, n), 0);
        assert_eq!(arrow_window_start(ARROW_ABOVE + 1, n), 1);
    }

    #[test]
    fn window_freezes_at_bottom_when_selection_near_end() {
        let n = 20;
        let max_start = n - ARROW_VISIBLE;
        assert_eq!(arrow_window_start(n - 1, n), max_start);
        assert_eq!(arrow_window_start(n - 2, n), max_start);
        assert_eq!(arrow_window_start(n - ARROW_VISIBLE + ARROW_ABOVE, n), max_start);
    }

    #[test]
    fn window_starts_at_zero_when_below_viewport() {
        assert_eq!(arrow_window_start(2, 5), 0);
        assert_eq!(arrow_window_start(0, 5), 0);
        assert_eq!(arrow_window_start(4, 5), 0);
    }

    #[test]
    fn window_starts_at_zero_when_exactly_viewport_size() {
        assert_eq!(arrow_window_start(3, ARROW_VISIBLE), 0);
        assert_eq!(arrow_window_start(ARROW_VISIBLE - 1, ARROW_VISIBLE), 0);
    }
}
