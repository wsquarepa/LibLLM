use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use libllm::group_chat::{
    ChatMode, CharacterAttachment, TALKATIVENESS_NOTCHES, normalized_talkativeness,
    notch_to_talkativeness, talkativeness_to_notch,
};
use libllm::session::{MessageTree, Session};

use crate::tui::theme::Theme;

pub struct ChatSettingsDialog {
    pub selected: usize,
    pub rows: Vec<Row>,
    pub button_focus: ButtonFocus,
    snapshot: Snapshot,
}

#[derive(Debug)]
pub enum Row {
    Scenario,
    Mode,
    Talkativeness { index: usize },
    Buttons,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonFocus {
    Cancel,
    Save,
}

#[derive(Debug)]
pub enum ChatSettingsAction {
    Continue,
    Save,
    Cancel,
    EditScenario,
}

#[derive(Debug, Clone)]
struct Snapshot {
    scenario: Option<String>,
    chat_mode: ChatMode,
    talkativeness: Vec<(f32, f32)>,
}

impl Snapshot {
    fn capture(session: &Session) -> Self {
        Self {
            scenario: session.scenario.clone(),
            chat_mode: session.chat_mode,
            talkativeness: session
                .characters
                .iter()
                .map(|c| (c.talkativeness, c.action_points))
                .collect(),
        }
    }

    fn restore(&self, session: &mut Session) {
        session.scenario = self.scenario.clone();
        session.chat_mode = self.chat_mode;
        for (i, (talk, action)) in self.talkativeness.iter().enumerate() {
            if let Some(c) = session.characters.get_mut(i) {
                c.talkativeness = *talk;
                c.action_points = *action;
            }
        }
    }
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
        rows.push(Row::Buttons);
        Self {
            selected: 0,
            rows,
            button_focus: ButtonFocus::Save,
            snapshot: Snapshot::capture(session),
        }
    }

    pub fn restore_snapshot(&self, session: &mut Session) {
        self.snapshot.restore(session);
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
        let dim_sliders = sliders_disabled(session.chat_mode);
        match key.code {
            KeyCode::Up => {
                self.move_selection(-1, dim_sliders);
                ChatSettingsAction::Continue
            }
            KeyCode::Down => {
                self.move_selection(1, dim_sliders);
                ChatSettingsAction::Continue
            }
            KeyCode::Left => {
                if matches!(self.rows[self.selected], Row::Buttons) {
                    self.button_focus = ButtonFocus::Cancel;
                } else {
                    self.adjust(session, -1, mode_locked, talkativeness_locked, set_locked_warning);
                }
                ChatSettingsAction::Continue
            }
            KeyCode::Right => {
                if matches!(self.rows[self.selected], Row::Buttons) {
                    self.button_focus = ButtonFocus::Save;
                } else {
                    self.adjust(session, 1, mode_locked, talkativeness_locked, set_locked_warning);
                }
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
                Row::Buttons => match self.button_focus {
                    ButtonFocus::Cancel => ChatSettingsAction::Cancel,
                    ButtonFocus::Save => ChatSettingsAction::Save,
                },
                _ => ChatSettingsAction::Continue,
            },
            KeyCode::Esc => ChatSettingsAction::Cancel,
            _ => ChatSettingsAction::Continue,
        }
    }

    fn move_selection(&mut self, delta: i32, dim_sliders: bool) {
        let len = self.rows.len();
        if len == 0 {
            return;
        }
        let mut i = self.selected as i32;
        loop {
            let next = i + delta;
            if next < 0 || next >= len as i32 {
                break;
            }
            i = next;
            let skip = dim_sliders && matches!(self.rows[i as usize], Row::Talkativeness { .. });
            if !skip {
                self.selected = i as usize;
                break;
            }
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
            Row::Scenario | Row::Buttons => {}
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
                if sliders_disabled(session.chat_mode) {
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
        let dim_sliders = sliders_disabled(session.chat_mode);
        let weights = normalized_talkativeness(&session.characters);
        let has_sliders = self
            .rows
            .iter()
            .any(|r| matches!(r, Row::Talkativeness { .. }));

        let slider_margin_lines = if has_sliders { 2 } else { 0 };
        let buttons_margin_lines = 1;
        let content_height =
            self.rows.len() as u16 + 4 + slider_margin_lines + buttons_margin_lines;
        let notches_total_u16 = TALKATIVENESS_NOTCHES as u16;
        let row_width = notches_total_u16 + 35;
        let preferred = (area.width as f32 * 0.7) as u16;
        let width = preferred.max(row_width).min(area.width);
        let dialog =
            super::super::render::clear_centered(f, width, content_height, area);

        let mut lines: Vec<Line> = vec![Line::from("")];
        let mut prev_was_slider = false;

        for (i, row) in self.rows.iter().enumerate() {
            let is_slider = matches!(row, Row::Talkativeness { .. });
            if is_slider && !prev_was_slider {
                lines.push(Line::from(""));
            }
            if !is_slider && prev_was_slider {
                lines.push(Line::from(""));
            }
            if matches!(row, Row::Buttons) {
                lines.push(Line::from(""));
            }

            let highlight = i == self.selected;
            let base_style = if highlight {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let line = match row {
                Row::Scenario => render_scenario_line(session, scenario_locked, base_style),
                Row::Mode => render_mode_line(session.chat_mode, base_style),
                Row::Talkativeness { index } => render_talkativeness_line(
                    &session.characters[*index],
                    weights.get(*index).copied().unwrap_or(0.0),
                    notches_total,
                    dim_sliders,
                    base_style,
                    theme,
                ),
                Row::Buttons => render_buttons_line(self.button_focus, highlight, width),
            };
            lines.push(line);
            prev_was_slider = is_slider;
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
                "Up/Down: navigate  Left/Right: adjust  Enter: confirm  Esc: cancel",
            )],
        );
    }
}

fn sliders_disabled(mode: ChatMode) -> bool {
    matches!(mode, ChatMode::RoundRobin | ChatMode::Directed)
}

fn render_scenario_line(
    session: &Session,
    scenario_locked: bool,
    base_style: Style,
) -> Line<'static> {
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

fn render_mode_line(mode: ChatMode, base_style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled("  mode:     ", base_style),
        Span::styled(mode.as_str().to_owned(), base_style),
    ])
}

fn render_talkativeness_line(
    character: &CharacterAttachment,
    weight: f32,
    notches_total: usize,
    dim_sliders: bool,
    base_style: Style,
    theme: &Theme,
) -> Line<'static> {
    let filled = talkativeness_to_notch(character.talkativeness) as usize;
    let bar: String = "#".repeat(filled) + &".".repeat(notches_total - filled);
    let percent = (weight * 100.0).round() as u32;
    let style = if dim_sliders {
        Style::default().fg(theme.dimmed)
    } else {
        base_style
    };
    Line::from(vec![Span::styled(
        format!(
            "  {:<16} [{bar}] {filled}/{notches_total}  ({percent:>3}%)",
            character.slug,
        ),
        style,
    )])
}

fn render_buttons_line(focus: ButtonFocus, highlight: bool, _dialog_width: u16) -> Line<'static> {
    let cancel_style = button_style(focus == ButtonFocus::Cancel && highlight);
    let save_style = button_style(focus == ButtonFocus::Save && highlight);
    Line::from(vec![
        Span::styled(" Cancel ", cancel_style),
        Span::raw("   "),
        Span::styled(" Save ", save_style),
        Span::raw("  "),
    ])
    .alignment(Alignment::Right)
}

fn button_style(selected: bool) -> Style {
    if selected {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}
