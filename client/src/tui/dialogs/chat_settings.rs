use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use libllm::group_chat::{
    ChatMode, TALKATIVENESS_NOTCHES, normalized_talkativeness, notch_to_talkativeness,
    talkativeness_to_notch,
};
use libllm::session::{MessageTree, Session};

use crate::tui::theme::Theme;

pub struct ChatSettingsDialog {
    pub selected: usize,
    pub rows: Vec<Row>,
}

#[derive(Debug)]
pub enum Row {
    Scenario,
    Mode,
    Talkativeness { index: usize },
}

#[derive(Debug)]
pub enum ChatSettingsAction {
    Continue,
    Close,
    EditScenario,
}

/// Reset session fields to their empty defaults, cancelling a provisional group creation.
///
/// Called when the user dismisses the chat-settings dialog without providing a scenario.
/// Clears the character list, chat mode, and scenario so the session is indistinguishable
/// from a freshly initialised one.
pub fn roll_back_provisional_group(session: &mut Session) {
    session.tree = MessageTree::new();
    session.characters.clear();
    session.chat_mode = ChatMode::default();
    session.scenario = None;
}

impl ChatSettingsDialog {
    pub fn for_session(session: &Session) -> Self {
        let mut rows = vec![Row::Scenario];
        if session.characters.len() >= 2 {
            rows.push(Row::Mode);
            for i in 0..session.characters.len() {
                rows.push(Row::Talkativeness { index: i });
            }
        }
        Self { selected: 0, rows }
    }

    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        session: &mut Session,
        scenario_locked: bool,
        mode_locked: bool,
        talkativeness_locked: &std::collections::HashMap<String, f32>,
        set_locked_warning: &mut Option<String>,
    ) -> ChatSettingsAction {
        match key.code {
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                ChatSettingsAction::Continue
            }
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1));
                ChatSettingsAction::Continue
            }
            KeyCode::Left => {
                self.adjust(session, -1, mode_locked, talkativeness_locked, set_locked_warning);
                ChatSettingsAction::Continue
            }
            KeyCode::Right => {
                self.adjust(session, 1, mode_locked, talkativeness_locked, set_locked_warning);
                ChatSettingsAction::Continue
            }
            KeyCode::Enter => match self.rows[self.selected] {
                Row::Scenario => {
                    if scenario_locked {
                        *set_locked_warning =
                            Some("scenario is locked by --scenario CLI flag".to_owned());
                        ChatSettingsAction::Continue
                    } else {
                        ChatSettingsAction::EditScenario
                    }
                }
                _ => ChatSettingsAction::Continue,
            },
            KeyCode::Esc => ChatSettingsAction::Close,
            _ => ChatSettingsAction::Continue,
        }
    }

    fn adjust(
        &self,
        session: &mut Session,
        notch_delta: i32,
        mode_locked: bool,
        talkativeness_locked: &std::collections::HashMap<String, f32>,
        set_locked_warning: &mut Option<String>,
    ) {
        match self.rows[self.selected] {
            Row::Scenario => {}
            Row::Mode => {
                if mode_locked {
                    *set_locked_warning =
                        Some("chat mode is locked by --chat-mode CLI flag".to_owned());
                    return;
                }
                session.chat_mode = match (session.chat_mode, notch_delta > 0) {
                    (ChatMode::ActionValue, true) => ChatMode::RoundRobin,
                    (ChatMode::RoundRobin, true) => ChatMode::WeightedRandom,
                    (ChatMode::WeightedRandom, true) => ChatMode::Directed,
                    (ChatMode::Directed, true) => ChatMode::ActionValue,
                    (ChatMode::Directed, false) => ChatMode::WeightedRandom,
                    (ChatMode::WeightedRandom, false) => ChatMode::RoundRobin,
                    (ChatMode::RoundRobin, false) => ChatMode::ActionValue,
                    (ChatMode::ActionValue, false) => ChatMode::Directed,
                };
            }
            Row::Talkativeness { index } => {
                if matches!(
                    session.chat_mode,
                    ChatMode::RoundRobin | ChatMode::Directed
                ) {
                    return;
                }
                let slug = session.characters[index].slug.clone();
                if talkativeness_locked.contains_key(&slug) {
                    *set_locked_warning = Some(format!(
                        "talkativeness for {slug} is locked by --talkativeness CLI flag"
                    ));
                    return;
                }
                let current =
                    talkativeness_to_notch(session.characters[index].talkativeness) as i32;
                let max = TALKATIVENESS_NOTCHES as i32;
                let new_notch = (current + notch_delta).clamp(0, max) as u8;
                let new_talk = notch_to_talkativeness(new_notch);
                session.characters[index].talkativeness = new_talk;
                session.characters[index].action_points =
                    libllm::group_chat::base_action_value(new_talk);
            }
        }
    }

    pub fn render(
        &self,
        f: &mut Frame,
        area: Rect,
        session: &Session,
        theme: &Theme,
        scenario_locked: bool,
    ) {
        let notches_total = TALKATIVENESS_NOTCHES as usize;
        let dim_sliders = matches!(
            session.chat_mode,
            ChatMode::RoundRobin | ChatMode::Directed
        );
        let weights = normalized_talkativeness(&session.characters);

        let content_height = self.rows.len() as u16 + 4;
        let notches_total_u16 = TALKATIVENESS_NOTCHES as u16;
        let row_width = notches_total_u16 + 35;
        let preferred = (area.width as f32 * 0.7) as u16;
        let width = preferred.max(row_width).min(area.width);
        let dialog =
            super::super::render::clear_centered(f, width, content_height, area);

        let mut lines: Vec<Line> = vec![Line::from("")];

        for (i, row) in self.rows.iter().enumerate() {
            let highlight = i == self.selected;
            let base_style = if highlight {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let line = match row {
                Row::Scenario => {
                    let preview = session
                        .scenario
                        .as_deref()
                        .map(|s| {
                            let trimmed = s.trim();
                            let char_count = trimmed.chars().count();
                            if trimmed.is_empty() {
                                "(empty \u{2014} press Enter)".to_owned()
                            } else if char_count <= 80 {
                                trimmed.to_owned()
                            } else {
                                let truncated: String = trimmed.chars().take(80).collect();
                                format!("{truncated}\u{2026}")
                            }
                        })
                        .unwrap_or_else(|| "(empty \u{2014} press Enter)".to_owned());
                    let scenario_style = if scenario_locked {
                        Style::default().fg(Color::Red)
                    } else {
                        base_style
                    };
                    Line::from(vec![
                        Span::styled("  Scenario: ", scenario_style),
                        Span::styled(preview, scenario_style),
                    ])
                }
                Row::Mode => {
                    let label_style = if highlight {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    Line::from(vec![
                        Span::styled("  mode:     ", label_style),
                        Span::styled(session.chat_mode.as_str().to_owned(), base_style),
                    ])
                }
                Row::Talkativeness { index } => {
                    let c = &session.characters[*index];
                    let filled = talkativeness_to_notch(c.talkativeness) as usize;
                    let bar: String =
                        "#".repeat(filled) + &".".repeat(notches_total - filled);
                    let percent = (weights.get(*index).copied().unwrap_or(0.0) * 100.0)
                        .round() as u32;
                    let style = if dim_sliders {
                        Style::default().fg(theme.dimmed)
                    } else {
                        base_style
                    };
                    Line::from(vec![Span::styled(
                        format!(
                            "  {:<16} [{bar}] {filled}/{notches_total}  ({percent:>3}%)",
                            c.slug,
                        ),
                        style,
                    )])
                }
            };
            lines.push(line);
        }

        let para = Paragraph::new(lines).block(
            super::super::render::dialog_block(" Chat Settings ", Color::Yellow),
        );
        f.render_widget(para, dialog);

        super::super::render::render_hints_below_dialog(
            f,
            dialog,
            area,
            &[Line::from(
                "Up/Down: navigate  Left/Right: adjust  Enter: edit scenario  Esc: save & close",
            )],
        );
    }
}
