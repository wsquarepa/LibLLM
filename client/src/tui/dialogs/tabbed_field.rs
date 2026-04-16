//! Tabbed multi-section field-editor dialog used by /config and /theme.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::Instant;
use tui_textarea::TextArea;

use super::validation::FieldValidation;

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
}
