//! Renders the auto-template-detect popup that asks the user whether to switch to
//! a better-matching instruct preset.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::theme::Theme;
use crate::tui::types::TemplatePromptState;

use super::{clear_centered, dialog_block};

#[derive(Debug, Clone, Copy)]
pub enum TemplatePromptResult {
    Pending,
    Switch,
    Dismiss,
}

#[expect(dead_code, reason = "wired in Task T18 (renderer + dialog handler routing)")]
pub(in crate::tui) fn render_template_prompt(
    f: &mut Frame,
    area: Rect,
    state: &TemplatePromptState,
    theme: &Theme,
) {
    let width = area.width.min(78).saturating_sub(2);
    let height = if state.expanded {
        area.height.min(24)
    } else {
        area.height.min(18)
    };
    let popup = clear_centered(f, width, height, area);

    let title = Span::styled(
        "Template mismatch",
        Style::default()
            .fg(theme.status_warning_bg)
            .add_modifier(Modifier::BOLD),
    );
    let inner = {
        let block = dialog_block(title, theme.status_warning_bg);
        let inner = block.inner(popup);
        f.render_widget(block, popup);
        inner
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(inner);

    let suggestion = if state.is_best_guess {
        format!(
            "  {}  (best guess — {:.0}% match)",
            state.suggested_preset.name,
            state.score * 100.0
        )
    } else {
        format!("  {}", state.suggested_preset.name)
    };
    let header_lines = vec![
        Line::from("Server uses a template that matches:"),
        Line::from(Span::styled(
            suggestion,
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];
    f.render_widget(Paragraph::new(header_lines), layout[0]);

    let body_width = layout[1].width as usize;
    let content_lines = preset_summary_lines(&state.suggested_preset, body_width, state.expanded);
    f.render_widget(
        Paragraph::new(content_lines).wrap(Wrap { trim: false }),
        layout[1],
    );

    let q = Line::from(format!(
        "Switch to \"{}\"?",
        state.suggested_preset.name
    ));
    let yes_style = if state.button_selected == 0 {
        Style::default()
            .fg(theme.nav_cursor_fg)
            .bg(theme.nav_cursor_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let no_style = if state.button_selected == 1 {
        Style::default()
            .fg(theme.nav_cursor_fg)
            .bg(theme.nav_cursor_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let buttons = Line::from(vec![
        Span::raw("                                  "),
        Span::styled(" Yes ", yes_style),
        Span::raw("  "),
        Span::styled(" No ", no_style),
    ]);
    f.render_widget(
        Paragraph::new(vec![q, Line::raw(""), buttons]).alignment(Alignment::Left),
        layout[2],
    );
}

fn preset_summary_lines(
    preset: &libllm::preset::InstructPreset,
    width: usize,
    expanded: bool,
) -> Vec<Line<'static>> {
    let stop_display = match &preset.stop_sequence {
        libllm::preset::StopSequence::Single(s) => s.clone(),
        libllm::preset::StopSequence::Multiple(v) => v.join(" | "),
    };

    let candidates: [(&str, String); 7] = [
        ("input_sequence", preset.input_sequence.clone()),
        ("output_sequence", preset.output_sequence.clone()),
        ("system_sequence", preset.system_sequence.clone()),
        ("stop_sequence", stop_display),
        ("input_suffix", preset.input_suffix.clone()),
        ("output_suffix", preset.output_suffix.clone()),
        ("system_suffix", preset.system_suffix.clone()),
    ];

    let mut out: Vec<Line<'static>> = vec![Line::from("Suggested preset:")];
    for (label, value) in candidates {
        if value.is_empty() {
            continue;
        }
        let displayed = if !expanded {
            let max = width.saturating_sub(label.len() + 6);
            if value.chars().count() <= max {
                value
            } else {
                let truncated: String = value.chars().take(max.saturating_sub(1)).collect();
                format!("{truncated}\u{2026}")
            }
        } else {
            value
        };
        out.push(Line::from(format!("    {label}: {displayed}")));
    }
    out
}

#[cfg_attr(not(test), expect(dead_code, reason = "wired in Task T18 (renderer + dialog handler routing)"))]
pub(in crate::tui) fn handle_template_prompt_key(
    key: KeyEvent,
    state: &mut TemplatePromptState,
) -> TemplatePromptResult {
    match key.code {
        KeyCode::Left | KeyCode::Right => {
            state.button_selected = 1 - state.button_selected;
            TemplatePromptResult::Pending
        }
        KeyCode::Tab => {
            state.expanded = !state.expanded;
            TemplatePromptResult::Pending
        }
        KeyCode::Enter => {
            if state.button_selected == 0 {
                TemplatePromptResult::Switch
            } else {
                TemplatePromptResult::Dismiss
            }
        }
        KeyCode::Esc => TemplatePromptResult::Dismiss,
        _ => TemplatePromptResult::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn fixture_state() -> TemplatePromptState {
        TemplatePromptState {
            suggested_preset: libllm::preset::resolve_instruct_preset("ChatML"),
            score: 0.99,
            is_best_guess: false,
            server_template_hash: "abc123".to_owned(),
            button_selected: 1,
            expanded: false,
        }
    }

    #[test]
    fn arrow_keys_toggle_button() {
        let mut s = fixture_state();
        assert_eq!(s.button_selected, 1);
        let _ = handle_template_prompt_key(
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            &mut s,
        );
        assert_eq!(s.button_selected, 0);
    }

    #[test]
    fn enter_on_yes_returns_switch() {
        let mut s = fixture_state();
        s.button_selected = 0;
        let r = handle_template_prompt_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut s,
        );
        assert!(matches!(r, TemplatePromptResult::Switch));
    }

    #[test]
    fn enter_on_no_returns_dismiss() {
        let mut s = fixture_state();
        s.button_selected = 1;
        let r = handle_template_prompt_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut s,
        );
        assert!(matches!(r, TemplatePromptResult::Dismiss));
    }

    #[test]
    fn esc_returns_dismiss() {
        let mut s = fixture_state();
        let r = handle_template_prompt_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &mut s,
        );
        assert!(matches!(r, TemplatePromptResult::Dismiss));
    }

    #[test]
    fn tab_toggles_expanded() {
        let mut s = fixture_state();
        assert!(!s.expanded);
        let _ = handle_template_prompt_key(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            &mut s,
        );
        assert!(s.expanded);
    }
}
