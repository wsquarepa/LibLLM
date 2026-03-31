pub mod api_error;
pub mod branch;
pub mod character;
pub mod delete_confirm;
pub mod edit;
pub mod passkey;
pub mod persona;
pub mod set_passkey;
pub mod system_prompt;
pub mod worldbook;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use tui_textarea::TextArea;

use crate::crypto::DerivedKey;
use crate::tui::BackgroundEvent;

use super::render::{clear_centered, dialog_block};

pub(in crate::tui) const MAX_TXT_IMPORT_BYTES: u64 = 1_024_000;
const MAX_IMPORT_NAME_LENGTH: usize = 64;

pub(in crate::tui) fn generate_unique_name(
    base: &str,
    existing: &std::collections::HashSet<String>,
) -> String {
    if !existing.contains(base) {
        return base.to_owned();
    }
    let mut i = 1u32;
    loop {
        let candidate = format!("{base}-{i}");
        if !existing.contains(&candidate) {
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
    if trimmed.is_empty() {
        return None;
    }
    let truncated = match trimmed.char_indices().nth(MAX_IMPORT_NAME_LENGTH) {
        Some((byte_idx, _)) => &trimmed[..byte_idx],
        None => trimmed,
    };
    Some(truncated.to_owned())
}

const MULTILINE_WIDTH_PERCENT: u16 = 70;
const MULTILINE_HEIGHT_PERCENT: u16 = 60;

const DIALOG_WIDTH_RATIO: f32 = 0.7;
const DIALOG_HEIGHT_RATIO: f32 = 0.6;
const LIST_DIALOG_WIDTH: u16 = 50;
const LIST_DIALOG_SHORT_PADDING: u16 = 6;
const LIST_DIALOG_TALL_PADDING: u16 = 7;
const FIELD_DIALOG_DEFAULT_WIDTH: u16 = 60;
const FIELD_DIALOG_PADDING_ROWS: u16 = 4;
const FIELD_DIALOG_EDITOR_EXTRA: u16 = 8;
const API_ERROR_DIALOG_WIDTH: u16 = 60;
const API_ERROR_DIALOG_HEIGHT: u16 = 8;
const LOADING_DIALOG_WIDTH: u16 = 40;
const LOADING_DIALOG_HEIGHT: u16 = 5;

const CONFIG_FIELDS: &[&str] = &[
    "API URL",
    "Template",
    "Temperature",
    "Top-K",
    "Top-P",
    "Min-P",
    "Repeat Last N",
    "Repeat Penalty",
    "Max Tokens",
    "TLS Skip Verify",
    "Debug Logging",
];
const CONFIG_BOOLEAN_FIELDS: &[usize] = &[9, 10];

const PERSONA_FIELDS: &[&str] = &["Name", "Persona"];
const PERSONA_MULTILINE: &[usize] = &[1];

const CHARACTER_EDITOR_FIELDS: &[&str] = &[
    "Name",
    "Description",
    "Personality",
    "Scenario",
    "First Message",
    "Examples",
    "System Prompt",
    "Post-History",
];
const CHARACTER_EDITOR_MULTILINE: &[usize] = &[1, 2, 3, 4, 5, 6, 7];

const SYSTEM_PROMPT_FIELDS: &[&str] = &["Name", "Content"];
const SYSTEM_PROMPT_MULTILINE: &[usize] = &[1];

const ENTRY_EDITOR_FIELDS: &[&str] = &[
    "Keys [OR]",
    "Content",
    "Selective",
    "Keys [AND]",
    "Constant",
    "Enabled",
    "Order",
    "Depth",
    "Case Sensitive",
];
const ENTRY_EDITOR_MULTILINE: &[usize] = &[1];
const ENTRY_EDITOR_PLACEHOLDER_FIELDS: &[usize] = &[0, 3];

pub fn open_config_editor(
    values: Vec<String>,
    locked_fields: Vec<usize>,
) -> FieldDialog<'static> {
    FieldDialog::new(" Configuration ", CONFIG_FIELDS, values, &[])
        .with_boolean_fields(CONFIG_BOOLEAN_FIELDS)
        .with_locked_fields(locked_fields)
}

pub fn open_persona_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(" Edit Persona ", PERSONA_FIELDS, values, PERSONA_MULTILINE)
}

pub fn open_character_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Character ",
        CHARACTER_EDITOR_FIELDS,
        values,
        CHARACTER_EDITOR_MULTILINE,
    )
}

pub fn open_system_prompt_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit System Prompt ",
        SYSTEM_PROMPT_FIELDS,
        values,
        SYSTEM_PROMPT_MULTILINE,
    )
}

pub fn open_entry_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Entry ",
        ENTRY_EDITOR_FIELDS,
        values,
        ENTRY_EDITOR_MULTILINE,
    )
    .with_placeholder("keyword1, keyword2, ...", ENTRY_EDITOR_PLACEHOLDER_FIELDS)
}

pub fn open_entry_editor_non_selective(values: Vec<String>) -> FieldDialog<'static> {
    let mut dialog = open_entry_editor(values);
    dialog.hidden_fields = vec![3];
    dialog
}

pub enum FieldDialogAction {
    Continue,
    Close,
}

pub struct FieldDialog<'a> {
    title: &'static str,
    labels: &'static [&'static str],
    pub values: Vec<String>,
    selected: usize,
    editing: bool,
    multiline_fields: &'static [usize],
    editor: Option<TextArea<'a>>,
    width: Option<u16>,
    height: Option<u16>,
    placeholder: Option<(&'static str, &'static [usize])>,
    boolean_fields: &'static [usize],
    pub hidden_fields: Vec<usize>,
    locked_fields: Vec<usize>,
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
        Self {
            title,
            labels,
            values,
            selected: 0,
            editing: false,
            multiline_fields,
            editor: None,
            width,
            height,
            placeholder: None,
            boolean_fields: &[],
            hidden_fields: Vec::new(),
            locked_fields: Vec::new(),
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

    fn open_multiline_editor(&mut self) {
        let content = &self.values[self.selected];
        let lines: Vec<String> = content.lines().map(String::from).collect();
        let mut editor = TextArea::from(if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        });
        super::configure_textarea_at_end(&mut editor);
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
            self.render_with_editor(f, dialog);
        } else {
            self.render_fields(f, dialog);
        }
    }

    fn render_fields(&self, f: &mut ratatui::Frame, dialog: Rect) {
        let mut lines: Vec<Line> = vec![Line::from("")];

        for (i, &label) in self.labels.iter().enumerate() {
            if self.hidden_fields.contains(&i) {
                continue;
            }
            let value = &self.values[i];
            let is_selected = i == self.selected;
            let cursor = if is_selected && self.editing { "_" } else { "" };

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

            let value_style = if is_locked {
                Style::default().fg(Color::Red)
            } else if is_selected {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };

            let is_empty = value.is_empty();
            let display_value = if self.is_boolean(i) {
                if value == "true" { "[x]".to_owned() } else { "[ ]".to_owned() }
            } else if self.is_multiline(i) && value.contains('\n') {
                format!("({} lines)", value.lines().count())
            } else {
                value.clone()
            };

            let has_placeholder = self
                .placeholder
                .is_some_and(|(_, fields)| fields.contains(&i));
            let show_placeholder = is_empty && !self.editing && has_placeholder;

            let value_span = if show_placeholder {
                let ph_text = self.placeholder.unwrap().0;
                Span::styled(ph_text, Style::default().fg(Color::DarkGray))
            } else {
                Span::styled(format!("{display_value}{cursor}"), value_style)
            };

            lines.push(Line::from(vec![
                Span::styled(format!("  {label:<17}"), label_style),
                value_span,
            ]));
        }

        lines.push(Line::from(""));
        let hint = if self.is_boolean(self.selected) {
            "  Up/Down: navigate  Enter: toggle  Esc: save & close"
        } else {
            "  Up/Down: navigate  Enter: edit  Esc: save & close"
        };
        lines.push(Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        )));

        let paragraph =
            Paragraph::new(Text::from(lines)).block(dialog_block(self.title, Color::Yellow));

        f.render_widget(paragraph, dialog);
    }

    fn render_with_editor(&self, f: &mut ratatui::Frame, dialog: Rect) {
        let editor = self.editor.as_ref().unwrap();
        let label = self.labels[self.selected];

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
        let hint_line = Line::from(Span::styled(
            "  Esc: done editing",
            Style::default().fg(Color::DarkGray),
        ));

        let header = Paragraph::new(Text::from(vec![Line::from(""), title_line]));
        let header_area = Rect { height: 2, ..inner };
        f.render_widget(header, header_area);

        let editor_area = Rect {
            x: inner.x + 1,
            y: inner.y + 2,
            width: inner.width.saturating_sub(2),
            height: inner.height.saturating_sub(4),
        };
        f.render_widget(editor, editor_area);

        let hint_area = Rect {
            x: inner.x,
            y: inner.y + inner.height - 1,
            width: inner.width,
            height: 1,
        };
        f.render_widget(Paragraph::new(hint_line), hint_area);

        f.render_widget(dialog_block(self.title, Color::Yellow), dialog);
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
                    editor.input(key);
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
                    self.values[self.selected].push(c);
                }
                KeyCode::Backspace => {
                    self.values[self.selected].pop();
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
                if !self.hidden_fields.contains(&self.selected) {
                    break;
                }
            },
            KeyCode::Down => loop {
                if self.selected >= self.labels.len() - 1 {
                    break;
                }
                self.selected += 1;
                if !self.hidden_fields.contains(&self.selected) {
                    break;
                }
            },
            KeyCode::Enter => {
                if self.is_locked(self.selected) {
                    // locked by CLI flag, no editing
                } else if self.is_boolean(self.selected) {
                    self.toggle_boolean();
                } else if self.is_multiline(self.selected) {
                    self.open_multiline_editor();
                } else {
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
}

fn log_phase(kind: &str, phase: &str, result: &str, elapsed: std::time::Duration) {
    crate::debug_log::log_kv(
        "unlock.phase",
        &[
            crate::debug_log::field("kind", kind),
            crate::debug_log::field("phase", phase),
            crate::debug_log::field("result", result),
            crate::debug_log::field(
                "elapsed_ms",
                format!("{:.3}", elapsed.as_secs_f64() * 1000.0),
            ),
        ],
    );
}

fn log_phase_with_path(
    kind: &str,
    phase: &str,
    result: &str,
    elapsed: std::time::Duration,
    path: std::path::Display<'_>,
) {
    crate::debug_log::log_kv(
        "unlock.phase",
        &[
            crate::debug_log::field("kind", kind),
            crate::debug_log::field("phase", phase),
            crate::debug_log::field("result", result),
            crate::debug_log::field(
                "elapsed_ms",
                format!("{:.3}", elapsed.as_secs_f64() * 1000.0),
            ),
            crate::debug_log::field("path", path),
        ],
    );
}

fn log_phase_with_error(
    kind: &str,
    phase: &str,
    elapsed: std::time::Duration,
    error: &anyhow::Error,
) {
    crate::debug_log::log_kv(
        "unlock.phase",
        &[
            crate::debug_log::field("kind", kind),
            crate::debug_log::field("phase", phase),
            crate::debug_log::field("result", "error"),
            crate::debug_log::field(
                "elapsed_ms",
                format!("{:.3}", elapsed.as_secs_f64() * 1000.0),
            ),
            crate::debug_log::field("error", error),
        ],
    );
}

pub(in crate::tui) fn derive_key_blocking<F>(
    passkey: String,
    debug_kind: &str,
    apply: F,
) -> BackgroundEvent
where
    F: FnOnce(DerivedKey, &std::path::Path) -> BackgroundEvent,
{
    let total_start = std::time::Instant::now();
    let salt_path = crate::config::salt_path();
    let check_path = crate::config::key_check_path();

    let salt_start = std::time::Instant::now();
    let salt_result = crate::crypto::load_or_create_salt(&salt_path);
    log_phase_with_path(
        debug_kind,
        "salt",
        if salt_result.is_ok() { "ok" } else { "error" },
        salt_start.elapsed(),
        salt_path.display(),
    );
    let salt = match salt_result {
        Ok(salt) => salt,
        Err(err) => {
            log_phase_with_error(debug_kind, "blocking_total", total_start.elapsed(), &err);
            return BackgroundEvent::KeyDeriveFailed(err.to_string());
        }
    };

    let derive_start = std::time::Instant::now();
    let derive_result = crate::crypto::derive_key(&passkey, &salt);
    log_phase(
        debug_kind,
        "argon2",
        if derive_result.is_ok() { "ok" } else { "error" },
        derive_start.elapsed(),
    );
    let derived_key = match derive_result {
        Ok(key) => key,
        Err(err) => {
            log_phase_with_error(debug_kind, "blocking_total", total_start.elapsed(), &err);
            return BackgroundEvent::KeyDeriveFailed(err.to_string());
        }
    };

    let result = apply(derived_key, &check_path);
    log_phase(
        debug_kind,
        "blocking_total",
        "done",
        total_start.elapsed(),
    );
    result
}
