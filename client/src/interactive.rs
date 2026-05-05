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

const CIRCULAR_VISIBLE: usize = 9;
const CIRCULAR_ABOVE: usize = 4;

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

/// Indices of items shown in the [`circular_select`] viewport, in render order.
///
/// When `n > CIRCULAR_VISIBLE`, returns 9 indices with `sel` always at slot 4
/// (cursor centered, viewport rotates around it). Otherwise returns
/// `0..n` so all items are visible without scrolling.
fn circular_window(sel: usize, n: usize) -> Vec<usize> {
    if n > CIRCULAR_VISIBLE {
        let start = (sel + n - CIRCULAR_ABOVE) % n;
        (0..CIRCULAR_VISIBLE)
            .map(|offset| (start + offset) % n)
            .collect()
    } else {
        (0..n).collect()
    }
}

/// Selector that always shows up to 9 rows with the cursor centered and the
/// viewport rotating circularly around it. Up/Down wrap through the list.
///
/// Returns `Ok(None)` on Esc / Ctrl+C / `q`. `default` is clamped into range.
pub fn circular_select(prompt: &str, items: &[String], default: usize) -> Result<Option<usize>> {
    if items.is_empty() {
        return Ok(None);
    }

    let term = Term::stderr();
    if !term.is_term() {
        anyhow::bail!("circular_select requires a TTY");
    }

    let n = items.len();
    let mut sel = default.min(n - 1);
    let prompt_q = style("?".to_string()).for_stderr().yellow();
    let prompt_text = Style::new().for_stderr().bold();
    let cursor = style(">".to_string()).for_stderr().cyan().bold();
    let active_item = Style::new().for_stderr().cyan();

    let render = |sel_idx: usize| -> io::Result<()> {
        let window = circular_window(sel_idx, n);
        let cursor_slot = if n > CIRCULAR_VISIBLE { CIRCULAR_ABOVE } else { sel_idx };
        let mut buf = String::new();
        let _ = writeln!(buf, "{} {}", prompt_q, prompt_text.apply_to(prompt));
        for slot in 0..CIRCULAR_VISIBLE {
            match window.get(slot) {
                Some(&idx) if slot == cursor_slot => {
                    let _ = writeln!(buf, "{} {}", cursor, active_item.apply_to(&items[idx]));
                }
                Some(&idx) => {
                    let _ = writeln!(buf, "  {}", items[idx]);
                }
                None => buf.push('\n'),
            }
        }
        term.write_str(&buf)
    };

    term.hide_cursor().context("failed to hide cursor")?;
    render(sel).context("failed to render selector")?;

    let outcome = loop {
        match term.read_key().context("failed to read key")? {
            Key::ArrowDown | Key::Tab | Key::Char('j') => sel = (sel + 1) % n,
            Key::ArrowUp | Key::BackTab | Key::Char('k') => sel = (sel + n - 1) % n,
            Key::Enter => break Some(sel),
            Key::Escape | Key::CtrlC | Key::Char('q') => break None,
            _ => continue,
        }
        term.clear_last_lines(CIRCULAR_VISIBLE + 1)
            .context("failed to clear selector frame")?;
        render(sel).context("failed to render selector")?;
    };

    term.clear_last_lines(CIRCULAR_VISIBLE + 1)
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
        let n = 20;
        let window = circular_window(10, n);
        assert_eq!(window.len(), CIRCULAR_VISIBLE);
        assert_eq!(window[CIRCULAR_ABOVE], 10);
        assert_eq!(window, vec![6, 7, 8, 9, 10, 11, 12, 13, 14]);
    }

    #[test]
    fn window_wraps_circularly_at_top() {
        let n = 20;
        let window = circular_window(0, n);
        assert_eq!(window.len(), CIRCULAR_VISIBLE);
        assert_eq!(window[CIRCULAR_ABOVE], 0);
        assert_eq!(window, vec![16, 17, 18, 19, 0, 1, 2, 3, 4]);
    }

    #[test]
    fn window_wraps_circularly_at_bottom() {
        let n = 20;
        let window = circular_window(19, n);
        assert_eq!(window.len(), CIRCULAR_VISIBLE);
        assert_eq!(window[CIRCULAR_ABOVE], 19);
        assert_eq!(window, vec![15, 16, 17, 18, 19, 0, 1, 2, 3]);
    }

    #[test]
    fn window_returns_full_list_when_below_viewport() {
        let window = circular_window(2, 5);
        assert_eq!(window, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn window_treats_exactly_viewport_size_as_static() {
        let window = circular_window(3, CIRCULAR_VISIBLE);
        assert_eq!(window, (0..CIRCULAR_VISIBLE).collect::<Vec<_>>());
    }
}
