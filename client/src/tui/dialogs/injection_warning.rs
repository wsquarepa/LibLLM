//! Prompt-injection warning modal. Shown when a file attached via `@<path>`
//! contains the reserved `<<<FILE …>>>` or `<<<END …>>>` delimiter for
//! its own basename on its own line. Single-action: dismiss and return
//! focus to the input.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::tui::{Action, App, Focus};

pub(in crate::tui) const INJECTION_WARNING_DIALOG_WIDTH: u16 = 72;
pub(in crate::tui) const INJECTION_WARNING_DIALOG_HEIGHT: u16 = 11;

/// State carried by the dialog: the basename of the file and the
/// delimiter variant that collided. Kept on `App::injection_warning`.
pub struct InjectionWarning {
    pub basename: String,
    pub delimiter: &'static str,
}

pub(in crate::tui) fn open(
    app: &mut App<'_>,
    path: &std::path::Path,
    kind: libllm::files::DelimiterKind,
) {
    let basename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_owned();
    let delimiter = match kind {
        libllm::files::DelimiterKind::Start => "<<<FILE ...>>>",
        libllm::files::DelimiterKind::End => "<<<END ...>>>",
    };
    app.injection_warning = Some(InjectionWarning {
        basename,
        delimiter,
    });
    app.focus = crate::tui::Focus::InjectionWarningDialog;
}

pub(in crate::tui) fn render(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let dialog = clear_centered(
        f,
        INJECTION_WARNING_DIALOG_WIDTH,
        INJECTION_WARNING_DIALOG_HEIGHT,
        area,
    );

    let (basename, delimiter) = match app.injection_warning.as_ref() {
        Some(w) => (w.basename.as_str(), w.delimiter),
        None => ("", ""),
    };

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Prompt-injection guard triggered",
            Style::default().fg(Color::Red),
        )),
        Line::from(""),
        Line::from(format!(
            "  File \"{basename}\" contains a literal {delimiter} line."
        )),
        Line::from(""),
        Line::from("  Sending it could confuse the model into misreading"),
        Line::from("  the attachment boundaries. The send was refused."),
        Line::from(""),
        Line::from("  Rename the file or edit its body to avoid the"),
        Line::from("  delimiter, then try again."),
    ];

    let paragraph =
        Paragraph::new(Text::from(lines)).block(dialog_block(" Injection Warning ", Color::Red));

    f.render_widget(paragraph, dialog);

    render_hints_below_dialog(f, dialog, area, &[Line::from("Press Enter or Esc to dismiss")]);
}

pub(in crate::tui) fn handle_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    match key.code {
        KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ') => {
            app.injection_warning = None;
            app.focus = Focus::Input;
        }
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injection_warning_state_round_trip() {
        let w = InjectionWarning {
            basename: "evil.md".to_owned(),
            delimiter: "<<<FILE \u{2026}>>>",
        };
        assert_eq!(w.basename, "evil.md");
        assert_eq!(w.delimiter, "<<<FILE \u{2026}>>>");
    }
}
