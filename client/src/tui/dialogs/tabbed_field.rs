//! Tabbed multi-section field-editor dialog used by /config and /theme.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use std::time::Instant;
use tui_textarea::TextArea;

use super::is_flash_active;
use super::validation::FieldValidation;
use crate::tui::render::{centered_rect, clear_centered, dialog_block, render_hints_below_dialog};
use crate::tui::theme::{Theme, parse_color};

pub struct TabSection {
    pub title: &'static str,
    pub labels: &'static [&'static str],
    pub values: Vec<String>,
    pub original_values: Vec<String>,
    pub multiline_fields: &'static [usize],
    pub boolean_fields: &'static [usize],
    pub selector_fields: &'static [usize],
    pub action_fields: &'static [usize],
    pub separator_fields: &'static [usize],
    pub placeholder_fields: &'static [usize],
    pub placeholder_text: Option<&'static str>,
    pub locked_fields: Vec<usize>,
    pub validated_fields: Vec<(usize, FieldValidation)>,
    pub color_preview_fields: &'static [usize],
    pub selected: usize,
}

pub enum TabbedFieldAction {
    Continue,
    Close,
    OpenSelector { section: usize, field: usize },
    InvokeAction { section: usize, field: usize },
}

pub struct TabbedFieldDialog<'a> {
    title: &'static str,
    sections: Vec<TabSection>,
    current_tab: usize,
    editing: bool,
    cursor_pos: usize,
    editor: Option<TextArea<'a>>,
    pub reject_flash: Option<Instant>,
    pub clipboard_warning: Option<String>,
}

impl<'a> TabbedFieldDialog<'a> {
    pub fn new(title: &'static str, sections: Vec<TabSection>) -> Self {
        Self {
            title,
            sections,
            current_tab: 0,
            editing: false,
            cursor_pos: 0,
            editor: None,
            reject_flash: None,
            clipboard_warning: None,
        }
    }

    pub fn current_tab(&self) -> usize {
        self.current_tab
    }

    pub fn sections(&self) -> &[TabSection] {
        &self.sections
    }

    pub fn has_changes(&self) -> bool {
        self.sections
            .iter()
            .any(|s| s.values != s.original_values)
    }

    fn next_tab(&mut self) {
        if self.sections.is_empty() {
            return;
        }
        self.current_tab = (self.current_tab + 1) % self.sections.len();
    }

    fn prev_tab(&mut self) {
        if self.sections.is_empty() {
            return;
        }
        self.current_tab = if self.current_tab == 0 {
            self.sections.len() - 1
        } else {
            self.current_tab - 1
        };
    }

    pub fn is_editing(&self) -> bool {
        self.editing || self.editor.is_some()
    }

    fn is_locked(&self, section: usize, field: usize) -> bool {
        self.sections[section].locked_fields.contains(&field)
    }

    fn is_boolean(&self, section: usize, field: usize) -> bool {
        self.sections[section].boolean_fields.contains(&field)
    }

    fn is_selector(&self, section: usize, field: usize) -> bool {
        self.sections[section].selector_fields.contains(&field)
    }

    fn is_action(&self, section: usize, field: usize) -> bool {
        self.sections[section].action_fields.contains(&field)
    }

    fn is_multiline(&self, section: usize, field: usize) -> bool {
        self.sections[section].multiline_fields.contains(&field)
    }

    fn is_color_preview(&self, section: usize, field: usize) -> bool {
        self.sections[section].color_preview_fields.contains(&field)
    }

    fn is_separator(&self, section: usize, field: usize) -> bool {
        self.sections[section].separator_fields.contains(&field)
    }

    fn validation_for(&self, section: usize, field: usize) -> Option<FieldValidation> {
        self.sections[section]
            .validated_fields
            .iter()
            .find(|(i, _)| *i == field)
            .map(|(_, v)| *v)
    }

    fn toggle_boolean(&mut self) {
        let tab = self.current_tab;
        let idx = self.sections[tab].selected;
        let val = &self.sections[tab].values[idx];
        self.sections[tab].values[idx] = if val == "true" {
            "false".to_owned()
        } else {
            "true".to_owned()
        };
    }

    fn open_multiline_editor(&mut self) {
        let tab = self.current_tab;
        let idx = self.sections[tab].selected;
        let content = &self.sections[tab].values[idx];
        let lines: Vec<String> = content.lines().map(String::from).collect();
        let mut editor = TextArea::from(if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        });
        crate::tui::dialog_handler::configure_textarea_at_end(&mut editor);
        self.editor = Some(editor);
    }

    fn move_selection_down(&mut self) {
        let tab = self.current_tab;
        let section = &mut self.sections[tab];
        let len = section.labels.len();
        if len == 0 {
            return;
        }
        loop {
            if section.selected + 1 >= len {
                return;
            }
            section.selected += 1;
            if !section.separator_fields.contains(&section.selected) {
                return;
            }
        }
    }

    fn move_selection_up(&mut self) {
        let tab = self.current_tab;
        let section = &mut self.sections[tab];
        loop {
            if section.selected == 0 {
                return;
            }
            section.selected -= 1;
            if !section.separator_fields.contains(&section.selected) {
                return;
            }
        }
    }

    pub fn insert_into_active_editor(&mut self, text: &str) {
        if let Some(ref mut editor) = self.editor {
            editor.insert_str(text);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> TabbedFieldAction {
        if let Some(ref mut editor) = self.editor {
            match key.code {
                KeyCode::Esc => {
                    let content = editor.lines().join("\n");
                    let tab = self.current_tab;
                    let idx = self.sections[tab].selected;
                    self.sections[tab].values[idx] = content;
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
            return TabbedFieldAction::Continue;
        }

        if self.editing {
            let tab = self.current_tab;
            let idx = self.sections[tab].selected;
            match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    let value = self.sections[tab].values[idx].clone();
                    let valid = self
                        .validation_for(tab, idx)
                        .map(|v| v.validate(&value))
                        .unwrap_or(true);
                    if valid {
                        self.editing = false;
                    } else {
                        self.reject_flash = Some(Instant::now());
                    }
                }
                KeyCode::Char(c) => {
                    let accept = self
                        .validation_for(tab, idx)
                        .map(|v| v.accepts_char(&self.sections[tab].values[idx], c))
                        .unwrap_or(true);
                    if accept {
                        let byte_pos = self.sections[tab].values[idx]
                            .char_indices()
                            .nth(self.cursor_pos)
                            .map(|(i, _)| i)
                            .unwrap_or(self.sections[tab].values[idx].len());
                        self.sections[tab].values[idx].insert(byte_pos, c);
                        self.cursor_pos += 1;
                    } else {
                        self.reject_flash = Some(Instant::now());
                    }
                }
                KeyCode::Backspace => {
                    if self.cursor_pos > 0 {
                        self.cursor_pos -= 1;
                        let byte_pos = self.sections[tab].values[idx]
                            .char_indices()
                            .nth(self.cursor_pos)
                            .map(|(i, _)| i)
                            .unwrap_or(self.sections[tab].values[idx].len());
                        self.sections[tab].values[idx].remove(byte_pos);
                    }
                }
                KeyCode::Delete => {
                    let char_count = self.sections[tab].values[idx].chars().count();
                    if self.cursor_pos < char_count {
                        let byte_pos = self.sections[tab].values[idx]
                            .char_indices()
                            .nth(self.cursor_pos)
                            .map(|(i, _)| i)
                            .unwrap_or(self.sections[tab].values[idx].len());
                        self.sections[tab].values[idx].remove(byte_pos);
                    }
                }
                KeyCode::Left => {
                    if self.cursor_pos > 0 {
                        self.cursor_pos -= 1;
                    }
                }
                KeyCode::Right => {
                    let char_count = self.sections[tab].values[idx].chars().count();
                    if self.cursor_pos < char_count {
                        self.cursor_pos += 1;
                    }
                }
                KeyCode::Home => self.cursor_pos = 0,
                KeyCode::End => {
                    self.cursor_pos = self.sections[tab].values[idx].chars().count();
                }
                _ => {}
            }
            return TabbedFieldAction::Continue;
        }

        match key.code {
            KeyCode::Tab => {
                self.next_tab();
            }
            KeyCode::BackTab => {
                self.prev_tab();
            }
            KeyCode::Char('\t') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.prev_tab();
            }
            KeyCode::Up => self.move_selection_up(),
            KeyCode::Down => self.move_selection_down(),
            KeyCode::Enter => {
                let tab = self.current_tab;
                let idx = self.sections[tab].selected;
                if self.is_locked(tab, idx) {
                    // locked by CLI override; no action
                } else if self.is_action(tab, idx) {
                    return TabbedFieldAction::InvokeAction {
                        section: tab,
                        field: idx,
                    };
                } else if self.is_selector(tab, idx) {
                    return TabbedFieldAction::OpenSelector {
                        section: tab,
                        field: idx,
                    };
                } else if self.is_boolean(tab, idx) {
                    self.toggle_boolean();
                } else if self.is_multiline(tab, idx) {
                    self.open_multiline_editor();
                } else {
                    self.cursor_pos =
                        self.sections[tab].values[idx].chars().count();
                    self.editing = true;
                }
            }
            KeyCode::Delete => {
                let tab = self.current_tab;
                let idx = self.sections[tab].selected;
                if self.is_color_preview(tab, idx) && !self.is_locked(tab, idx) {
                    self.sections[tab].values[idx].clear();
                }
            }
            KeyCode::Esc => {
                return TabbedFieldAction::Close;
            }
            _ => {}
        }
        TabbedFieldAction::Continue
    }

    const LABEL_PREFIX_WIDTH: usize = 24;
    const DIALOG_WIDTH: u16 = 78;
    const SWATCH_WIDTH: usize = 4;
    const MULTILINE_WIDTH_PERCENT: u16 = 70;
    const MULTILINE_HEIGHT_PERCENT: u16 = 60;

    fn dialog_dimensions(&self, area: Rect) -> (u16, u16) {
        if self.editor.is_some() {
            let w = (area.width as f32 * Self::MULTILINE_WIDTH_PERCENT as f32 / 100.0) as u16;
            let h = (area.height as f32 * Self::MULTILINE_HEIGHT_PERCENT as f32 / 100.0) as u16;
            return (w, h);
        }
        let tallest = self
            .sections
            .iter()
            .map(|s| s.labels.len() as u16)
            .max()
            .unwrap_or(0);
        let height = tallest + 6;
        (Self::DIALOG_WIDTH, height)
    }

    pub fn render(&self, f: &mut ratatui::Frame, area: Rect, theme: &Theme) {
        let (w, h) = self.dialog_dimensions(area);
        let dialog = clear_centered(f, w, h, area);
        if self.editor.is_some() {
            self.render_with_editor(f, dialog, area);
        } else {
            self.render_tabs_and_fields(f, dialog, area, theme);
        }
    }

    fn render_tabs_and_fields(
        &self,
        f: &mut ratatui::Frame,
        dialog: Rect,
        area: Rect,
        theme: &Theme,
    ) {
        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(""));
        lines.push(self.build_tab_bar_line());
        lines.push(Line::from(""));
        let section = &self.sections[self.current_tab];
        for (i, &label) in section.labels.iter().enumerate() {
            if section.separator_fields.contains(&i) {
                lines.push(Line::from(""));
                continue;
            }
            lines.push(self.build_field_line(section, i, label, dialog.width, theme));
        }
        let paragraph =
            Paragraph::new(Text::from(lines)).block(dialog_block(self.title, Color::Yellow));
        f.render_widget(paragraph, dialog);
        render_hints_below_dialog(f, dialog, area, &[Line::from(self.current_hint())]);
    }

    fn current_hint(&self) -> &'static str {
        let tab = self.current_tab;
        let idx = self.sections[tab].selected;
        if self.is_boolean(tab, idx) {
            "Tab/Shift-Tab: section  ↑/↓: field  Enter: toggle  Esc: save & close"
        } else if self.is_selector(tab, idx) {
            "Tab/Shift-Tab: section  ↑/↓: field  Enter: select  Esc: save & close"
        } else if self.is_action(tab, idx) {
            "Tab/Shift-Tab: section  ↑/↓: field  Enter: invoke  Esc: save & close"
        } else {
            "Tab/Shift-Tab: section  ↑/↓: field  Enter: edit  Esc: save & close"
        }
    }

    fn build_tab_bar_line(&self) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::raw("  ")];
        for (i, section) in self.sections.iter().enumerate() {
            let label = format!("[ {} ]", section.title);
            let style = if i == self.current_tab {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(label, style));
            spans.push(Span::raw(" "));
        }
        Line::from(spans)
    }

    fn build_field_line(
        &self,
        section: &TabSection,
        i: usize,
        label: &'static str,
        dialog_width: u16,
        theme: &Theme,
    ) -> Line<'static> {
        let value = &section.values[i];
        let is_selected = i == section.selected;
        let show_cursor = is_selected && self.editing;
        let is_locked = section.locked_fields.contains(&i);

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
        let is_color = section.color_preview_fields.contains(&i);
        let value_is_invalid_color =
            is_color && !value.is_empty() && parse_color(value).is_none();

        let value_style = if is_locked || value_is_invalid_color {
            Style::default().fg(Color::Red)
        } else if flashing {
            Style::default().fg(Color::Yellow)
        } else if is_selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };

        let is_empty = value.is_empty();
        let is_boolean = section.boolean_fields.contains(&i);
        let is_multiline = section.multiline_fields.contains(&i);
        let display_value = if is_boolean {
            if value == "true" {
                "[x]".to_owned()
            } else {
                "[ ]".to_owned()
            }
        } else if is_multiline && value.contains('\n') {
            format!("({} lines)", value.lines().count())
        } else {
            value.clone()
        };

        let has_placeholder = section.placeholder_fields.contains(&i) || is_color;
        let show_placeholder = is_empty && !self.editing && has_placeholder;

        let max_value_width = dialog_width as usize
            - 2
            - Self::LABEL_PREFIX_WIDTH
            - if is_color { Self::SWATCH_WIDTH + 1 } else { 0 };

        let mut spans: Vec<Span<'static>> =
            vec![Span::styled(format!("  {label:<22}"), label_style)];

        if is_color {
            let swatch_color = self.resolve_swatch_color(value, theme, label);
            let swatch_style = Style::default().fg(swatch_color);
            spans.push(Span::styled("████".to_owned(), swatch_style));
            spans.push(Span::raw(" "));
        }

        if show_placeholder {
            let ph_text = if is_color {
                "(inherit)"
            } else {
                section.placeholder_text.unwrap_or("")
            };
            spans.push(Span::styled(
                ph_text.to_owned(),
                Style::default().fg(Color::DarkGray),
            ));
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
        }

        Line::from(spans)
    }

    fn resolve_swatch_color(&self, value: &str, theme: &Theme, label: &str) -> Color {
        if value.is_empty() {
            return theme_color_by_label(theme, label);
        }
        parse_color(value).unwrap_or(theme.status_error_bg)
    }

    pub fn handle_mouse_click(
        &mut self,
        terminal_area: Rect,
        screen_col: u16,
        screen_row: u16,
    ) -> bool {
        let (w, h) = self.dialog_dimensions(terminal_area);
        let dialog = centered_rect(w, h, terminal_area);
        let pos = Position::new(screen_col, screen_row);
        if !dialog.contains(pos) {
            return false;
        }

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
            return true;
        }

        if self.editing {
            return true;
        }

        let tab_bar_y = dialog.y + 2;
        if screen_row == tab_bar_y {
            if let Some(target) = self.hit_test_tab_bar(dialog.x + 2, screen_col) {
                self.current_tab = target;
            }
            return true;
        }

        let fields_start_y = dialog.y + 4;
        if screen_row >= fields_start_y {
            let inner_row = (screen_row - fields_start_y) as usize;
            let section = &mut self.sections[self.current_tab];
            let mut visible_idx: usize = 0;
            for i in 0..section.labels.len() {
                if section.separator_fields.contains(&i) {
                    visible_idx += 1;
                    continue;
                }
                if visible_idx == inner_row {
                    section.selected = i;
                    return true;
                }
                visible_idx += 1;
            }
        }
        true
    }

    fn hit_test_tab_bar(&self, start_col: u16, click_col: u16) -> Option<usize> {
        let mut cursor = start_col;
        for (i, section) in self.sections.iter().enumerate() {
            let label_width = section.title.len() as u16 + 4;
            let end = cursor + label_width;
            if click_col >= cursor && click_col < end {
                return Some(i);
            }
            cursor = end + 1;
        }
        None
    }

    fn render_with_editor(&self, f: &mut ratatui::Frame, dialog: Rect, area: Rect) {
        let editor = self.editor.as_ref().unwrap();
        let tab = self.current_tab;
        let idx = self.sections[tab].selected;
        let label = self.sections[tab].labels[idx];
        let inner = Rect {
            x: dialog.x + 1,
            y: dialog.y + 1,
            width: dialog.width.saturating_sub(2),
            height: dialog.height.saturating_sub(2),
        };
        let title_line = Line::from(Span::styled(
            format!("  Editing: {label}"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        let header = Paragraph::new(Text::from(vec![Line::from(""), title_line]));
        let header_area = Rect { height: 2, ..inner };
        f.render_widget(header, header_area);
        let editor_area = Rect {
            x: inner.x + 1,
            y: inner.y + 2,
            width: inner.width.saturating_sub(2),
            height: inner.height.saturating_sub(3),
        };
        f.render_widget(editor, editor_area);
        f.render_widget(dialog_block(self.title, Color::Yellow), dialog);
        render_hints_below_dialog(f, dialog, area, &[Line::from("Esc: done editing")]);
    }
}

fn theme_color_by_label(theme: &Theme, label: &str) -> Color {
    match label {
        "user_message" => theme.user_message,
        "assistant_message_fg" => theme.assistant_message_fg,
        "assistant_message_bg" => theme.assistant_message_bg,
        "system_message" => theme.system_message,
        "border_focused" => theme.border_focused,
        "border_unfocused" => theme.border_unfocused,
        "status_bar_fg" => theme.status_bar_fg,
        "status_bar_bg" => theme.status_bar_bg,
        "status_error_fg" => theme.status_error_fg,
        "status_error_bg" => theme.status_error_bg,
        "status_info_fg" => theme.status_info_fg,
        "status_info_bg" => theme.status_info_bg,
        "status_warning_fg" => theme.status_warning_fg,
        "status_warning_bg" => theme.status_warning_bg,
        "dialogue" => theme.dialogue,
        "nav_cursor_fg" => theme.nav_cursor_fg,
        "nav_cursor_bg" => theme.nav_cursor_bg,
        "hover_bg" => theme.hover_bg,
        "dimmed" => theme.dimmed,
        "sidebar_highlight_fg" => theme.sidebar_highlight_fg,
        "sidebar_highlight_bg" => theme.sidebar_highlight_bg,
        "command_picker_fg" => theme.command_picker_fg,
        "command_picker_bg" => theme.command_picker_bg,
        "streaming_indicator" => theme.streaming_indicator,
        "api_unavailable" => theme.api_unavailable,
        "summary_indicator" => theme.summary_indicator,
        _ => Color::Reset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_section(title: &'static str, labels: &'static [&'static str]) -> TabSection {
        let values: Vec<String> = labels.iter().map(|_| String::new()).collect();
        let original_values = values.clone();
        TabSection {
            title,
            labels,
            values,
            original_values,
            multiline_fields: &[],
            boolean_fields: &[],
            selector_fields: &[],
            action_fields: &[],
            separator_fields: &[],
            placeholder_fields: &[],
            placeholder_text: None,
            locked_fields: Vec::new(),
            validated_fields: Vec::new(),
            color_preview_fields: &[],
            selected: 0,
        }
    }

    #[test]
    fn tab_wraps_forward() {
        let sections = vec![
            make_section("A", &["f"]),
            make_section("B", &["f"]),
            make_section("C", &["f"]),
        ];
        let mut d = TabbedFieldDialog::new(" test ", sections);
        assert_eq!(d.current_tab(), 0);
        d.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(d.current_tab(), 1);
        d.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(d.current_tab(), 2);
        d.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(d.current_tab(), 0);
    }

    #[test]
    fn tab_wraps_backward_via_backtab() {
        let sections = vec![
            make_section("A", &["f"]),
            make_section("B", &["f"]),
        ];
        let mut d = TabbedFieldDialog::new(" test ", sections);
        d.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(d.current_tab(), 1);
        d.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(d.current_tab(), 0);
    }

    #[test]
    fn tab_wraps_backward_via_shift_tab_char() {
        let sections = vec![
            make_section("A", &["f"]),
            make_section("B", &["f"]),
        ];
        let mut d = TabbedFieldDialog::new(" test ", sections);
        d.handle_key(KeyEvent::new(KeyCode::Char('\t'), KeyModifiers::SHIFT));
        assert_eq!(d.current_tab(), 1);
    }

    #[test]
    fn has_changes_detects_edits_in_any_section() {
        let sections = vec![
            make_section("A", &["f"]),
            make_section("B", &["f"]),
        ];
        let mut d = TabbedFieldDialog::new(" test ", sections);
        assert!(!d.has_changes());
        d.sections[1].values[0] = "changed".to_owned();
        assert!(d.has_changes());
    }

    #[test]
    fn down_arrow_moves_within_section_only() {
        let sections = vec![
            make_section("A", &["x", "y", "z"]),
            make_section("B", &["p", "q"]),
        ];
        let mut d = TabbedFieldDialog::new(" test ", sections);
        d.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(d.sections[0].selected, 1);
        d.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(d.sections[0].selected, 2);
        d.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(d.sections[0].selected, 2);
        assert_eq!(d.sections[1].selected, 0);
    }

    #[test]
    fn tab_disabled_while_editing() {
        let sections = vec![
            make_section("A", &["x"]),
            make_section("B", &["y"]),
        ];
        let mut d = TabbedFieldDialog::new(" test ", sections);
        d.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(d.is_editing());
        d.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(d.current_tab(), 0);
    }

    #[test]
    fn boolean_toggles_on_enter() {
        let labels: &'static [&'static str] = &["Flag"];
        let mut section = make_section("A", labels);
        section.boolean_fields = {
            const B: &[usize] = &[0];
            B
        };
        section.values[0] = "false".to_owned();
        let mut d = TabbedFieldDialog::new(" test ", vec![section]);
        d.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(d.sections[0].values[0], "true");
        d.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(d.sections[0].values[0], "false");
    }

    #[test]
    fn action_field_dispatches_invoke_action() {
        let labels: &'static [&'static str] = &["Reset"];
        let mut section = make_section("A", labels);
        section.action_fields = {
            const A: &[usize] = &[0];
            A
        };
        let mut d = TabbedFieldDialog::new(" test ", vec![section]);
        let action = d.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match action {
            TabbedFieldAction::InvokeAction { section, field } => {
                assert_eq!(section, 0);
                assert_eq!(field, 0);
            }
            _ => panic!("expected InvokeAction"),
        }
    }

    #[test]
    fn selector_field_dispatches_open_selector() {
        let labels: &'static [&'static str] = &["Preset"];
        let mut section = make_section("A", labels);
        section.selector_fields = {
            const S: &[usize] = &[0];
            S
        };
        let mut d = TabbedFieldDialog::new(" test ", vec![section]);
        let action = d.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match action {
            TabbedFieldAction::OpenSelector { section, field } => {
                assert_eq!(section, 0);
                assert_eq!(field, 0);
            }
            _ => panic!("expected OpenSelector"),
        }
    }

    #[test]
    fn invalid_color_rejected_on_commit() {
        let labels: &'static [&'static str] = &["Color"];
        let mut section = make_section("A", labels);
        section.validated_fields = vec![(0, FieldValidation::Color)];
        let mut d = TabbedFieldDialog::new(" test ", vec![section]);
        d.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        for c in "notacolor".chars() {
            d.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        d.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(d.is_editing());
        assert!(d.reject_flash.is_some());
    }

    #[test]
    fn valid_color_accepted_on_commit() {
        let labels: &'static [&'static str] = &["Color"];
        let mut section = make_section("A", labels);
        section.validated_fields = vec![(0, FieldValidation::Color)];
        let mut d = TabbedFieldDialog::new(" test ", vec![section]);
        d.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        for c in "#ff0000".chars() {
            d.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        d.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!d.is_editing());
        assert_eq!(d.sections[0].values[0], "#ff0000");
    }

    #[test]
    fn delete_key_clears_color_field_with_value() {
        let labels: &'static [&'static str] = &["Color"];
        let mut section = make_section("A", labels);
        section.color_preview_fields = {
            const C: &[usize] = &[0];
            C
        };
        section.validated_fields = vec![(0, FieldValidation::Color)];
        section.values[0] = "#ff0000".to_owned();
        let mut d = TabbedFieldDialog::new(" test ", vec![section]);
        d.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(d.sections[0].values[0], "");
    }

    #[test]
    fn esc_at_top_level_closes() {
        let sections = vec![make_section("A", &["x"])];
        let mut d = TabbedFieldDialog::new(" test ", sections);
        let action = d.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(action, TabbedFieldAction::Close));
    }

    #[test]
    fn tab_bar_highlights_current_tab() {
        let sections = vec![
            make_section("Alpha", &["x"]),
            make_section("Beta", &["y"]),
        ];
        let mut d = TabbedFieldDialog::new(" test ", sections);
        let line = d.build_tab_bar_line();
        let rendered: String = line
            .spans
            .iter()
            .map(|s| s.content.as_ref().to_string())
            .collect();
        assert!(rendered.contains("Alpha"));
        assert!(rendered.contains("Beta"));

        d.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        let line_after = d.build_tab_bar_line();
        let styles: Vec<Style> = line_after.spans.iter().map(|s| s.style).collect();
        assert!(styles.iter().any(|s| s.fg == Some(Color::Yellow)));
    }

    #[test]
    fn theme_color_lookup_returns_expected() {
        let theme = Theme::dark();
        assert_eq!(theme_color_by_label(&theme, "user_message"), Color::Green);
        assert_eq!(theme_color_by_label(&theme, "unknown"), Color::Reset);
    }

    #[test]
    fn mouse_click_on_tab_bar_switches_section() {
        let sections = vec![
            make_section("A", &["x"]),
            make_section("B", &["y"]),
            make_section("C", &["z"]),
        ];
        let mut d = TabbedFieldDialog::new(" test ", sections);
        let area = Rect::new(0, 0, 80, 20);
        let (w, h) = d.dialog_dimensions(area);
        let dialog_x = (area.width.saturating_sub(w)) / 2;
        let dialog_y = (area.height.saturating_sub(h)) / 2;
        let tab_bar_y = dialog_y + 2;
        let b_col = dialog_x + 2 + "[ A ] ".len() as u16 + 2;
        let handled = d.handle_mouse_click(area, b_col, tab_bar_y);
        assert!(handled);
        assert_eq!(d.current_tab(), 1);
    }

    #[test]
    fn mouse_click_on_field_row_selects_field() {
        let sections = vec![make_section("A", &["first", "second", "third"])];
        let mut d = TabbedFieldDialog::new(" test ", sections);
        let area = Rect::new(0, 0, 80, 20);
        let (w, h) = d.dialog_dimensions(area);
        let dialog_x = (area.width.saturating_sub(w)) / 2;
        let dialog_y = (area.height.saturating_sub(h)) / 2;
        let fields_start_y = dialog_y + 4;
        let handled = d.handle_mouse_click(area, dialog_x + 5, fields_start_y + 1);
        assert!(handled);
        assert_eq!(d.sections[0].selected, 1);
    }
}
