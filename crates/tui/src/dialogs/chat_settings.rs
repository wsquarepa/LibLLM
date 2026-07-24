use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use libllm_core::group_chat::{
    CharacterAttachment, ChatMode, normalized_talkativeness, notch_to_talkativeness,
    talkativeness_notches, talkativeness_to_notch,
};
use libllm_core::session::{MessageTree, Session};

use crate::theme::Theme;

/// Tri-state view of the provisional scenario staged by the child editor.
///
/// Distinguishes "editor never opened" from "editor opened and cleared", which the flat
/// `provisional_scenario()` getter collapses to `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvisionalScenario {
    /// Scenario editor has not been opened this dialog session.
    Absent,
    /// Editor was opened and closed with an empty field (user cleared the scenario).
    Cleared,
    /// Editor was opened and closed with non-empty text.
    Text(String),
}

pub struct ChatSettingsDialog {
    pub selected: usize,
    pub rows: Vec<Row>,
    pub button_focus: ButtonFocus,
    // `None` = scenario editor was never opened; `Some(v)` = editor was opened and closed,
    // with `v` holding the text (or `None` if the user cleared the field).
    provisional_scenario: Option<Option<String>>,
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
            provisional_scenario: None,
            snapshot: Snapshot::capture(session),
        }
    }

    pub fn restore_snapshot(&self, session: &mut Session) {
        self.snapshot.restore(session);
    }

    /// Stores scenario text typed in the child editor without writing it to the session.
    ///
    /// Called by the scenario editor on close, even when the user cleared the field (passes
    /// `None`). The outer `Some` records that the editor was opened; the inner value holds the
    /// text.
    pub fn set_provisional_scenario(&mut self, value: Option<String>) {
        self.provisional_scenario = Some(value);
    }

    /// Returns the pending provisional scenario text for pre-populating the editor on reopen.
    ///
    /// Returns `None` both when the editor has never been opened and when the user cleared
    /// the field. Prefer [`provisional_scenario_state`] when those two states must differ.
    pub fn provisional_scenario(&self) -> Option<&str> {
        self.provisional_scenario.as_ref()?.as_deref()
    }

    /// Returns the full tri-state provisional scenario (never opened / cleared / text).
    pub fn provisional_scenario_state(&self) -> ProvisionalScenario {
        match &self.provisional_scenario {
            None => ProvisionalScenario::Absent,
            Some(None) => ProvisionalScenario::Cleared,
            Some(Some(s)) => ProvisionalScenario::Text(s.clone()),
        }
    }

    /// Writes the provisional scenario into the session. Called only on Save.
    ///
    /// When the editor was never opened (`provisional_scenario` is `None`), the existing
    /// `session.scenario` is left untouched. When the editor was opened and closed, the
    /// inner value (which may itself be `None` when the user cleared the field) is written.
    pub fn commit_provisional_scenario(&mut self, session: &mut Session) {
        if let Some(value) = self.provisional_scenario.take() {
            session.scenario = value;
        }
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
                    self.adjust(
                        session,
                        -1,
                        mode_locked,
                        talkativeness_locked,
                        set_locked_warning,
                    );
                }
                ChatSettingsAction::Continue
            }
            KeyCode::Right => {
                if matches!(self.rows[self.selected], Row::Buttons) {
                    self.button_focus = ButtonFocus::Save;
                } else {
                    self.adjust(
                        session,
                        1,
                        mode_locked,
                        talkativeness_locked,
                        set_locked_warning,
                    );
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
                let notches = talkativeness_notches(session.characters.len());
                let current =
                    talkativeness_to_notch(session.characters[index].talkativeness, notches) as i32;
                let new_notch = (current + notch_delta).clamp(0, notches as i32) as u8;
                let new_talk = notch_to_talkativeness(new_notch, notches);
                session.characters[index].talkativeness = new_talk;
                session.characters[index].action_points =
                    libllm_core::group_chat::base_action_value(new_talk);
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
        let notches_total = talkativeness_notches(session.characters.len()) as usize;
        let dim_sliders = sliders_disabled(session.chat_mode);
        let weights = normalized_talkativeness(&session.characters);
        let has_sliders = self
            .rows
            .iter()
            .any(|r| matches!(r, Row::Talkativeness { .. }));
        let has_mode = self.rows.iter().any(|r| matches!(r, Row::Mode));

        let slider_margin_lines = if has_sliders { 2 } else { 0 };
        let buttons_margin_lines = 1;
        let content_height =
            self.rows.len() as u16 + 4 + slider_margin_lines + buttons_margin_lines;
        let row_width = notches_total as u16 + 35;
        let mode_width = if has_mode {
            mode_line_width() as u16 + 4
        } else {
            0
        };
        let preferred = (area.width as f32 * 0.7) as u16;
        let width = preferred.max(row_width).max(mode_width).min(area.width);
        let dialog = super::super::render::clear_centered(f, width, content_height, area);
        let content_width = width.saturating_sub(2) as usize;

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
                Row::Scenario => render_scenario_line(
                    match &self.provisional_scenario {
                        Some(v) => v.as_deref(),
                        None => session.scenario.as_deref(),
                    },
                    scenario_locked,
                    base_style,
                ),
                Row::Mode => render_mode_line(session.chat_mode, highlight),
                Row::Talkativeness { index } => render_talkativeness_line(
                    &session.characters[*index],
                    weights.get(*index).copied().unwrap_or(0.0),
                    notches_total,
                    content_width,
                    dim_sliders,
                    base_style,
                    theme,
                ),
                Row::Buttons => render_buttons_line(self.button_focus, highlight, width),
            };
            lines.push(line);
            prev_was_slider = is_slider;
        }

        let para = Paragraph::new(lines).block(super::super::render::dialog_block(
            " Chat Settings ",
            Color::Yellow,
        ));
        f.render_widget(para, dialog);

        super::super::render::render_hints_below_dialog(
            f,
            dialog,
            area,
            &[Line::from(self.current_hint())],
        );
    }

    fn current_hint(&self) -> &'static str {
        match self.rows[self.selected] {
            Row::Scenario => "Up/Down: navigate  Enter: edit  Esc: cancel",
            Row::Mode => "Up/Down: navigate  Left/Right: change mode  Esc: cancel",
            Row::Talkativeness { .. } => "Up/Down: navigate  Left/Right: adjust  Esc: cancel",
            Row::Buttons => "Up/Down: navigate  Left/Right: select  Enter: confirm  Esc: cancel",
        }
    }
}

fn sliders_disabled(mode: ChatMode) -> bool {
    matches!(mode, ChatMode::RoundRobin | ChatMode::Directed)
}

fn render_scenario_line(
    scenario: Option<&str>,
    scenario_locked: bool,
    base_style: Style,
) -> Line<'static> {
    let preview = scenario
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

const MODE_LABEL: &str = "  mode:     ";
const CHAT_MODES: [ChatMode; 4] = [
    ChatMode::ActionValue,
    ChatMode::RoundRobin,
    ChatMode::WeightedRandom,
    ChatMode::Directed,
];
const MODE_OPTION_SEPARATOR: &str = "  ";

/// Rendered width of the mode radio row, used to size the dialog so every option fits.
fn mode_line_width() -> usize {
    let radio_marker_width = 4;
    let options: usize = CHAT_MODES
        .iter()
        .map(|m| radio_marker_width + m.as_str().chars().count())
        .sum();
    let separators = MODE_OPTION_SEPARATOR.chars().count() * (CHAT_MODES.len() - 1);
    MODE_LABEL.chars().count() + options + separators
}

fn render_mode_line(mode: ChatMode, focused: bool) -> Line<'static> {
    let label_style = if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let mut spans: Vec<Span<'static>> = vec![Span::styled(MODE_LABEL, label_style)];
    for (i, option) in CHAT_MODES.iter().enumerate() {
        let selected = *option == mode;
        let marker = if selected { "(*) " } else { "( ) " };
        let style = match (selected, focused) {
            (true, true) => Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            (true, false) => Style::default().add_modifier(Modifier::BOLD),
            (false, true) => Style::default().fg(Color::Cyan),
            (false, false) => Style::default().fg(Color::DarkGray),
        };
        spans.push(Span::styled(format!("{marker}{}", option.as_str()), style));
        if i + 1 < CHAT_MODES.len() {
            spans.push(Span::raw(MODE_OPTION_SEPARATOR));
        }
    }
    Line::from(spans)
}

fn render_talkativeness_line(
    character: &CharacterAttachment,
    weight: f32,
    notches_total: usize,
    content_width: usize,
    dim_sliders: bool,
    base_style: Style,
    theme: &Theme,
) -> Line<'static> {
    let filled = talkativeness_to_notch(character.talkativeness, notches_total as u8) as usize;
    let bar: String = "#".repeat(filled) + &".".repeat(notches_total - filled);
    let percent = (weight * 100.0).round() as u32;
    let notch_digits = notches_total.to_string().len();
    let slider = format!("[{bar}] {filled:>notch_digits$}/{notches_total}  ({percent:>3}%)");
    let name = format!("  {}", character.slug);
    let right_edge = content_width.saturating_sub(1);
    let gap = right_edge
        .saturating_sub(name.chars().count() + slider.chars().count())
        .max(1);
    let style = if dim_sliders {
        Style::default().fg(theme.dimmed)
    } else {
        base_style
    };
    Line::from(vec![Span::styled(
        format!("{name}{}{slider}", " ".repeat(gap)),
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
