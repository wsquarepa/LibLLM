//! Modal dialog rendering and key handling for all TUI overlay panels.

pub mod api_error;
pub mod branch;
pub mod character;
pub mod delete_confirm;
pub mod edit;
pub mod passkey;
pub mod persona;
pub mod preset;
pub mod set_passkey;
pub mod system_prompt;
pub mod worldbook;

mod builders;
mod crypto;
mod mouse;
mod tabbed_field;
mod validation;

pub use builders::{
    open_character_editor, open_config_editor, open_entry_editor, open_entry_editor_non_selective,
    open_instruct_editor, open_persona_editor, open_reasoning_editor, open_system_prompt_editor,
    open_template_editor, open_theme_editor,
};
pub use tabbed_field::{TabSection, TabbedFieldAction, TabbedFieldDialog};
pub(in crate::tui) use builders::{
    DIALOG_HEIGHT_RATIO, DIALOG_WIDTH_RATIO, FIELD_DIALOG_DEFAULT_WIDTH, LIST_DIALOG_TALL_PADDING,
    LIST_DIALOG_WIDTH, THEME_COLOR_TAB_LAYOUT,
};
pub(in crate::tui) use crypto::derive_key_blocking;
use crypto::log_phase_with_path;
pub(in crate::tui) use mouse::handle_dialog_mouse_click;
pub use validation::FieldValidation;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use tui_textarea::TextArea;

use super::render::{clear_centered, dialog_block, render_hints_below_dialog};

pub(in crate::tui) const MAX_TXT_IMPORT_BYTES: u64 = 1_024_000;
pub(in crate::tui) const MAX_NAME_LENGTH: usize = 32;
const MAX_PASSKEY_LENGTH: usize = 128;

pub(in crate::tui) fn generate_unique_name(
    base: &str,
    existing: &std::collections::HashSet<String>,
) -> String {
    let lower_existing: std::collections::HashSet<String> =
        existing.iter().map(|s| s.to_lowercase()).collect();
    if !lower_existing.contains(&base.to_lowercase()) {
        return base.to_owned();
    }
    let mut i = 1u32;
    loop {
        let candidate = format!("{base}-{i}");
        if !lower_existing.contains(&candidate.to_lowercase()) {
            return candidate;
        }
        i += 1;
    }
}

pub(in crate::tui) fn move_selection_up(selected: &mut usize) {
    *selected = selected.saturating_sub(1);
}

pub(in crate::tui) fn move_selection_down(selected: &mut usize, list_len: usize) {
    if list_len > 0 {
        *selected = (*selected + 1).min(list_len - 1);
    }
}

pub(in crate::tui) fn sanitize_import_name(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_NAME_LENGTH {
        return None;
    }
    Some(trimmed.to_owned())
}

const REJECT_FLASH_DURATION: std::time::Duration = std::time::Duration::from_millis(150);

pub(in crate::tui) fn is_flash_active(flash: Option<std::time::Instant>) -> bool {
    flash.is_some_and(|t| t.elapsed() < REJECT_FLASH_DURATION)
}

const MULTILINE_WIDTH_PERCENT: u16 = 70;
const MULTILINE_HEIGHT_PERCENT: u16 = 60;

pub(in crate::tui) fn byte_pos_at_char(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

pub(in crate::tui) fn render_multiline_editor(
    f: &mut ratatui::Frame,
    dialog: ratatui::layout::Rect,
    area: ratatui::layout::Rect,
    editor: &tui_textarea::TextArea<'_>,
    label: &str,
    title: &'static str,
) {
    let inner = ratatui::layout::Rect {
        x: dialog.x + 1,
        y: dialog.y + 1,
        width: dialog.width.saturating_sub(2),
        height: dialog.height.saturating_sub(2),
    };
    let title_line = ratatui::text::Line::from(ratatui::text::Span::styled(
        format!("  Editing: {label}"),
        ratatui::style::Style::default()
            .fg(ratatui::style::Color::Yellow)
            .add_modifier(ratatui::style::Modifier::BOLD),
    ));
    let header = ratatui::widgets::Paragraph::new(ratatui::text::Text::from(vec![
        ratatui::text::Line::from(""),
        title_line,
    ]));
    let header_area = ratatui::layout::Rect { height: 2, ..inner };
    f.render_widget(header, header_area);
    let editor_area = ratatui::layout::Rect {
        x: inner.x + 1,
        y: inner.y + 2,
        width: inner.width.saturating_sub(2),
        height: inner.height.saturating_sub(3),
    };
    f.render_widget(editor, editor_area);
    f.render_widget(crate::tui::render::dialog_block(title, ratatui::style::Color::Yellow), dialog);
    crate::tui::render::render_hints_below_dialog(f, dialog, area, &[ratatui::text::Line::from("Esc: done editing")]);
}

const FIELD_DIALOG_PADDING_ROWS: u16 = 3;
const FIELD_DIALOG_EDITOR_EXTRA: u16 = 8;
const API_ERROR_DIALOG_WIDTH: u16 = 60;
const API_ERROR_DIALOG_HEIGHT: u16 = 6;
const LOADING_DIALOG_WIDTH: u16 = 40;
const LOADING_DIALOG_HEIGHT: u16 = 5;

pub enum FieldDialogAction {
    Continue,
    Close,
    OpenSelector(usize),
}

pub struct FieldDialog<'a> {
    title: &'static str,
    labels: &'static [&'static str],
    pub values: Vec<String>,
    pub original_values: Vec<String>,
    selected: usize,
    editing: bool,
    cursor_pos: usize,
    multiline_fields: &'static [usize],
    editor: Option<TextArea<'a>>,
    width: Option<u16>,
    height: Option<u16>,
    placeholder: Option<(&'static str, &'static [usize])>,
    boolean_fields: &'static [usize],
    pub hidden_fields: Vec<usize>,
    locked_fields: Vec<usize>,
    validated_fields: Vec<(usize, FieldValidation)>,
    separator_fields: &'static [usize],
    selector_fields: &'static [usize],
    pub reject_flash: Option<std::time::Instant>,
    pub clipboard_warning: Option<String>,
}

impl<'a> FieldDialog<'a> {
    fn new(
        title: &'static str,
        labels: &'static [&'static str],
        values: Vec<String>,
        multiline_fields: &'static [usize],
    ) -> Self {
        let (width, height) = if multiline_fields.is_empty() {
            (None, None)
        } else {
            (
                Some(MULTILINE_WIDTH_PERCENT),
                Some(MULTILINE_HEIGHT_PERCENT),
            )
        };
        let original_values = values.clone();
        Self {
            title,
            labels,
            values,
            original_values,
            selected: 0,
            editing: false,
            cursor_pos: 0,
            multiline_fields,
            editor: None,
            width,
            height,
            placeholder: None,
            boolean_fields: &[],
            hidden_fields: Vec::new(),
            locked_fields: Vec::new(),
            validated_fields: Vec::new(),
            separator_fields: &[],
            selector_fields: &[],
            reject_flash: None,
            clipboard_warning: None,
        }
    }

    fn with_placeholder(mut self, text: &'static str, fields: &'static [usize]) -> Self {
        self.placeholder = Some((text, fields));
        self
    }

    fn with_boolean_fields(mut self, fields: &'static [usize]) -> Self {
        self.boolean_fields = fields;
        self
    }

    pub(in crate::tui) fn with_locked_fields(mut self, fields: Vec<usize>) -> Self {
        self.locked_fields = fields;
        self
    }

    fn with_validated_fields(mut self, fields: Vec<(usize, FieldValidation)>) -> Self {
        self.validated_fields = fields;
        self
    }

    pub fn has_changes(&self) -> bool {
        self.values != self.original_values
    }

    fn is_locked(&self, index: usize) -> bool {
        self.locked_fields.contains(&index)
    }

    fn is_boolean(&self, index: usize) -> bool {
        self.boolean_fields.contains(&index)
    }

    fn toggle_boolean(&mut self) {
        let val = &self.values[self.selected];
        self.values[self.selected] = if val == "true" {
            "false".to_owned()
        } else {
            "true".to_owned()
        };
    }

    fn is_multiline(&self, index: usize) -> bool {
        self.multiline_fields.contains(&index)
    }

    fn is_separator(&self, index: usize) -> bool {
        self.separator_fields.contains(&index)
    }

    pub fn is_selector(&self, index: usize) -> bool {
        self.selector_fields.contains(&index)
    }

    fn validation_for(&self, index: usize) -> Option<FieldValidation> {
        self.validated_fields
            .iter()
            .find(|(i, _)| *i == index)
            .map(|(_, v)| *v)
    }

    pub fn insert_into_active_editor(&mut self, text: &str) {
        if let Some(ref mut editor) = self.editor {
            editor.insert_str(text);
        }
    }

    fn open_multiline_editor(&mut self) {
        let content = &self.values[self.selected];
        let lines: Vec<String> = content.lines().map(String::from).collect();
        let mut editor = TextArea::from(if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        });
        super::dialog_handler::configure_textarea_at_end(&mut editor);
        self.editor = Some(editor);
    }

    pub fn render(&self, f: &mut ratatui::Frame, area: Rect) {
        let default_height = self.labels.len() as u16 + FIELD_DIALOG_PADDING_ROWS;
        let (w, h) = match (self.width, self.height) {
            (Some(wp), Some(hp)) => {
                let w = (area.width as f32 * wp as f32 / 100.0) as u16;
                let h = (area.height as f32 * hp as f32 / 100.0) as u16;
                (w, h)
            }
            _ => {
                let editor_extra = if self.editor.is_some() {
                    FIELD_DIALOG_EDITOR_EXTRA
                } else {
                    0
                };
                (FIELD_DIALOG_DEFAULT_WIDTH, default_height + editor_extra)
            }
        };
        let dialog = clear_centered(f, w, h, area);

        if self.editor.is_some() {
            self.render_with_editor(f, dialog, area);
        } else {
            self.render_fields(f, dialog, area);
        }
    }

    const LABEL_PREFIX_WIDTH: usize = 24;

    fn render_fields(&self, f: &mut ratatui::Frame, dialog: Rect, area: Rect) {
        let mut lines: Vec<Line> = vec![Line::from("")];

        for (i, &label) in self.labels.iter().enumerate() {
            if self.hidden_fields.contains(&i) {
                continue;
            }
            if self.is_separator(i) {
                lines.push(Line::from(""));
                continue;
            }
            let value = &self.values[i];
            let is_selected = i == self.selected;
            let show_cursor = is_selected && self.editing;

            let is_locked = self.is_locked(i);

            let label_style = if is_locked {
                Style::default().fg(Color::Red)
            } else if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let flashing = is_selected && self.editing && is_flash_active(self.reject_flash);
            let value_style = if is_locked {
                Style::default().fg(Color::Red)
            } else if flashing {
                Style::default().fg(Color::Yellow)
            } else if is_selected {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };

            let is_empty = value.is_empty();
            let display_value = if self.is_boolean(i) {
                if value == "true" {
                    "[x]".to_owned()
                } else {
                    "[ ]".to_owned()
                }
            } else if self.is_multiline(i) && value.contains('\n') {
                format!("({} lines)", value.lines().count())
            } else {
                value.clone()
            };

            let has_placeholder = self
                .placeholder
                .is_some_and(|(_, fields)| fields.contains(&i));
            let show_placeholder = is_empty && !self.editing && has_placeholder;

            let max_value_width = dialog.width as usize - 2 - Self::LABEL_PREFIX_WIDTH;
            let mut spans = vec![Span::styled(format!("  {label:<22}"), label_style)];

            if show_placeholder {
                let ph_text = self.placeholder.unwrap().0;
                spans.push(Span::styled(ph_text, Style::default().fg(Color::DarkGray)));
            } else if show_cursor {
                let chars: Vec<char> = display_value.chars().collect();
                let char_count = chars.len();
                let visible_width = max_value_width.saturating_sub(1);
                let scroll = if self.cursor_pos > visible_width {
                    self.cursor_pos - visible_width
                } else {
                    0
                };
                let visible_end = (scroll + max_value_width).min(char_count);

                let before: String = chars[scroll..self.cursor_pos].iter().collect();
                let cursor_ch = if self.cursor_pos < char_count {
                    chars[self.cursor_pos].to_string()
                } else {
                    " ".to_string()
                };
                let after_start = (self.cursor_pos + 1).min(char_count);
                let after: String = chars[after_start..visible_end].iter().collect();

                let cursor_style = value_style.add_modifier(Modifier::REVERSED);
                spans.push(Span::styled(before, value_style));
                spans.push(Span::styled(cursor_ch, cursor_style));
                spans.push(Span::styled(after, value_style));
            } else {
                let full = &display_value;
                let visible: String = if full.chars().count() > max_value_width {
                    let skip = full.chars().count() - max_value_width;
                    full.chars().skip(skip).collect()
                } else {
                    full.clone()
                };
                spans.push(Span::styled(visible, value_style));
            };

            lines.push(Line::from(spans));
        }

        let paragraph =
            Paragraph::new(Text::from(lines)).block(dialog_block(self.title, Color::Yellow));

        f.render_widget(paragraph, dialog);

        let hint = if self.is_boolean(self.selected) {
            "Up/Down: navigate  Enter: toggle  Esc: save & close"
        } else if self.is_selector(self.selected) {
            "Up/Down: navigate  Enter: select  Esc: save & close"
        } else {
            "Up/Down: navigate  Enter: edit  Esc: save & close"
        };
        render_hints_below_dialog(f, dialog, area, &[Line::from(hint)]);
    }

    fn render_with_editor(&self, f: &mut ratatui::Frame, dialog: Rect, area: Rect) {
        let editor = self.editor.as_ref().unwrap();
        let label = self.labels[self.selected];
        render_multiline_editor(f, dialog, area, editor, label, self.title);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> FieldDialogAction {
        if let Some(ref mut editor) = self.editor {
            match key.code {
                KeyCode::Esc => {
                    let content = editor.lines().join("\n");
                    self.values[self.selected] = content;
                    self.editor = None;
                }
                _ => {
                    let (consumed, warning) =
                        crate::tui::clipboard::handle_clipboard_key(&key, editor);
                    self.clipboard_warning = warning;
                    if !consumed {
                        editor.input(key);
                    }
                }
            }
            return FieldDialogAction::Continue;
        }

        if self.editing {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    self.editing = false;
                }
                KeyCode::Char(c) => {
                    let accept = self
                        .validation_for(self.selected)
                        .map(|v| v.accepts_char(&self.values[self.selected], c))
                        .unwrap_or(true);
                    if accept {
                        let byte_pos = byte_pos_at_char(&self.values[self.selected], self.cursor_pos);
                        self.values[self.selected].insert(byte_pos, c);
                        self.cursor_pos += 1;
                    } else {
                        self.reject_flash = Some(std::time::Instant::now());
                    }
                }
                KeyCode::Backspace => {
                    if self.cursor_pos > 0 {
                        self.cursor_pos -= 1;
                        let byte_pos = byte_pos_at_char(&self.values[self.selected], self.cursor_pos);
                        self.values[self.selected].remove(byte_pos);
                    }
                }
                KeyCode::Delete => {
                    let char_count = self.values[self.selected].chars().count();
                    if self.cursor_pos < char_count {
                        let byte_pos = byte_pos_at_char(&self.values[self.selected], self.cursor_pos);
                        self.values[self.selected].remove(byte_pos);
                    }
                }
                KeyCode::Left => {
                    if self.cursor_pos > 0 {
                        self.cursor_pos -= 1;
                    }
                }
                KeyCode::Right => {
                    let char_count = self.values[self.selected].chars().count();
                    if self.cursor_pos < char_count {
                        self.cursor_pos += 1;
                    }
                }
                KeyCode::Home => {
                    self.cursor_pos = 0;
                }
                KeyCode::End => {
                    self.cursor_pos = self.values[self.selected].chars().count();
                }
                _ => {}
            }
            return FieldDialogAction::Continue;
        }

        match key.code {
            KeyCode::Up => loop {
                if self.selected == 0 {
                    break;
                }
                self.selected -= 1;
                if !self.hidden_fields.contains(&self.selected) && !self.is_separator(self.selected)
                {
                    break;
                }
            },
            KeyCode::Down => loop {
                if self.selected >= self.labels.len() - 1 {
                    break;
                }
                self.selected += 1;
                if !self.hidden_fields.contains(&self.selected) && !self.is_separator(self.selected)
                {
                    break;
                }
            },
            KeyCode::Enter => {
                if self.is_locked(self.selected) {
                    // locked by CLI flag, no editing
                } else if self.is_selector(self.selected) {
                    return FieldDialogAction::OpenSelector(self.selected);
                } else if self.is_boolean(self.selected) {
                    self.toggle_boolean();
                } else if self.is_multiline(self.selected) {
                    self.open_multiline_editor();
                } else {
                    self.cursor_pos = self.values[self.selected].chars().count();
                    self.editing = true;
                }
            }
            KeyCode::Esc => {
                return FieldDialogAction::Close;
            }
            _ => {}
        }

        FieldDialogAction::Continue
    }

    pub fn handle_mouse_click(
        &mut self,
        terminal_area: Rect,
        screen_col: u16,
        screen_row: u16,
    ) -> bool {
        let (w, h) = self.dialog_dimensions(terminal_area);
        let dialog = super::render::centered_rect(w, h, terminal_area);
        let pos = Position::new(screen_col, screen_row);
        if !dialog.contains(pos) {
            return false;
        }

        if self.editor.is_some() {
            if let Some(ref mut editor) = self.editor {
                editor.input(crossterm::event::Event::Mouse(
                    crossterm::event::MouseEvent {
                        kind: crossterm::event::MouseEventKind::Down(
                            crossterm::event::MouseButton::Left,
                        ),
                        column: screen_col,
                        row: screen_row,
                        modifiers: crossterm::event::KeyModifiers::NONE,
                    },
                ));
            }
            return true;
        }

        if self.editing {
            return true;
        }

        let inner_row = screen_row.saturating_sub(dialog.y + 2);
        let mut visible_idx: u16 = 0;
        for (i, _label) in self.labels.iter().enumerate() {
            if self.hidden_fields.contains(&i) {
                continue;
            }
            if self.is_separator(i) {
                visible_idx += 1;
                continue;
            }
            if visible_idx == inner_row {
                self.selected = i;
                return true;
            }
            visible_idx += 1;
        }
        true
    }

    fn dialog_dimensions(&self, terminal_area: Rect) -> (u16, u16) {
        let default_height = self.labels.len() as u16 + FIELD_DIALOG_PADDING_ROWS;
        match (self.width, self.height) {
            (Some(wp), Some(hp)) => {
                let w = (terminal_area.width as f32 * wp as f32 / 100.0) as u16;
                let h = (terminal_area.height as f32 * hp as f32 / 100.0) as u16;
                (w, h)
            }
            _ => {
                let editor_extra = if self.editor.is_some() {
                    FIELD_DIALOG_EDITOR_EXTRA
                } else {
                    0
                };
                (FIELD_DIALOG_DEFAULT_WIDTH, default_height + editor_extra)
            }
        }
    }
}

