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

    pub fn handle_key(&mut self, key: KeyEvent) -> TabbedFieldAction {
        if self.editor.is_none() && !self.editing {
            match key.code {
                KeyCode::Tab => {
                    self.next_tab();
                    return TabbedFieldAction::Continue;
                }
                KeyCode::BackTab => {
                    self.prev_tab();
                    return TabbedFieldAction::Continue;
                }
                KeyCode::Char('\t') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.prev_tab();
                    return TabbedFieldAction::Continue;
                }
                _ => {}
            }
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
}
