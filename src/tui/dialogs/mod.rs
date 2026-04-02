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

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use tui_textarea::TextArea;

use crate::crypto::DerivedKey;
use crate::tui::BackgroundEvent;

use super::render::{clear_centered, dialog_block, render_hints_below_dialog};

pub(in crate::tui) const MAX_TXT_IMPORT_BYTES: u64 = 1_024_000;
pub(in crate::tui) const MAX_NAME_LENGTH: usize = 32;
const MAX_PASSKEY_LENGTH: usize = 128;

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

const DIALOG_WIDTH_RATIO: f32 = 0.7;
const DIALOG_HEIGHT_RATIO: f32 = 0.6;
const LIST_DIALOG_WIDTH: u16 = 50;
const LIST_DIALOG_TALL_PADDING: u16 = 4;
const FIELD_DIALOG_DEFAULT_WIDTH: u16 = 60;
const FIELD_DIALOG_PADDING_ROWS: u16 = 3;
const FIELD_DIALOG_EDITOR_EXTRA: u16 = 8;
const API_ERROR_DIALOG_WIDTH: u16 = 60;
const API_ERROR_DIALOG_HEIGHT: u16 = 6;
const LOADING_DIALOG_WIDTH: u16 = 40;
const LOADING_DIALOG_HEIGHT: u16 = 5;

const CONFIG_FIELDS: &[&str] = &[
    "API URL",
    "",
    "Template",
    "Instruct",
    "Reasoning",
    "",
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
const CONFIG_BOOLEAN_FIELDS: &[usize] = &[13, 14];
const CONFIG_SEPARATOR_FIELDS: &[usize] = &[1, 5];
const CONFIG_SELECTOR_FIELDS: &[usize] = &[2, 3, 4];

const TEMPLATE_EDITOR_FIELDS: &[&str] = &[
    "Name",
    "Story String",
    "Example Separator",
    "Chat Start",
];
const TEMPLATE_EDITOR_MULTILINE: &[usize] = &[1];

const INSTRUCT_EDITOR_FIELDS: &[&str] = &[
    "Name",
    "Input Sequence",
    "Output Sequence",
    "System Sequence",
    "Input Suffix",
    "Output Suffix",
    "System Suffix",
    "Stop Sequence",
    "Separator Sequence",
    "Wrap",
    "System Same As User",
    "Seq. As Stop Strings",
];
const INSTRUCT_EDITOR_BOOLEAN: &[usize] = &[9, 10, 11];

const REASONING_EDITOR_FIELDS: &[&str] = &["Name", "Prefix", "Suffix", "Separator"];

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
        .with_separator_fields(CONFIG_SEPARATOR_FIELDS)
        .with_selector_fields(CONFIG_SELECTOR_FIELDS)
        .with_validated_fields(vec![
            (6, FieldValidation::Float { min: 0.0, max: 2.0 }),
            (7, FieldValidation::Int { min: 1, max: 100 }),
            (8, FieldValidation::Float { min: 0.0, max: 1.0 }),
            (9, FieldValidation::Float { min: 0.0, max: 1.0 }),
            (10, FieldValidation::Int { min: -1, max: 32767 }),
            (11, FieldValidation::Float { min: 0.0, max: 2.0 }),
            (12, FieldValidation::Int { min: -1, max: 32767 }),
        ])
}

pub fn open_persona_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(" Edit Persona ", PERSONA_FIELDS, values, PERSONA_MULTILINE)
        .with_validated_fields(vec![(0, FieldValidation::MaxLen(MAX_NAME_LENGTH))])
}

pub fn open_character_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Character ",
        CHARACTER_EDITOR_FIELDS,
        values,
        CHARACTER_EDITOR_MULTILINE,
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(MAX_NAME_LENGTH))])
}

pub fn open_template_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Template Preset ",
        TEMPLATE_EDITOR_FIELDS,
        values,
        TEMPLATE_EDITOR_MULTILINE,
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(MAX_NAME_LENGTH))])
}

pub fn open_instruct_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Instruct Preset ",
        INSTRUCT_EDITOR_FIELDS,
        values,
        &[],
    )
    .with_boolean_fields(INSTRUCT_EDITOR_BOOLEAN)
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(MAX_NAME_LENGTH))])
}

pub fn open_reasoning_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Reasoning Preset ",
        REASONING_EDITOR_FIELDS,
        values,
        &[],
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(MAX_NAME_LENGTH))])
}

pub fn open_system_prompt_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit System Prompt ",
        SYSTEM_PROMPT_FIELDS,
        values,
        SYSTEM_PROMPT_MULTILINE,
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(MAX_NAME_LENGTH))])
}

pub fn open_entry_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Entry ",
        ENTRY_EDITOR_FIELDS,
        values,
        ENTRY_EDITOR_MULTILINE,
    )
    .with_placeholder("keyword1, keyword2, ...", ENTRY_EDITOR_PLACEHOLDER_FIELDS)
    .with_validated_fields(vec![
        (6, FieldValidation::Int { min: -999, max: 999 }),
        (7, FieldValidation::Int { min: 0, max: 24 }),
    ])
}

pub fn open_entry_editor_non_selective(values: Vec<String>) -> FieldDialog<'static> {
    let mut dialog = open_entry_editor(values);
    dialog.hidden_fields = vec![3];
    dialog
}

#[derive(Clone, Copy)]
pub enum FieldValidation {
    Float { min: f64, max: f64 },
    Int { min: i64, max: i64 },
    MaxLen(usize),
}

impl FieldValidation {
    fn max_digits(max_abs: u64) -> usize {
        if max_abs == 0 { 1 } else { (max_abs as f64).log10().floor() as usize + 1 }
    }

    fn accepts_char(&self, current: &str, c: char) -> bool {
        match self {
            Self::Float { min, max } => {
                if c == '-' {
                    return *min < 0.0 && current.is_empty();
                }
                if c == '.' {
                    return !current.contains('.');
                }
                if !c.is_ascii_digit() {
                    return false;
                }
                let digits_only = current.trim_start_matches('-');
                if let Some(dot_pos) = digits_only.find('.') {
                    digits_only.len() - dot_pos <= 2
                } else {
                    let max_whole = Self::max_digits(max.abs() as u64);
                    digits_only.len() < max_whole
                }
            }
            Self::Int { min, max } => {
                if c == '-' {
                    *min < 0 && current.is_empty()
                } else if c.is_ascii_digit() {
                    let digits_only = current.trim_start_matches('-');
                    let max_abs = (*min).unsigned_abs().max((*max).unsigned_abs());
                    digits_only.len() < Self::max_digits(max_abs)
                } else {
                    false
                }
            }
            Self::MaxLen(max) => current.chars().count() < *max,
        }
    }
}

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

    fn with_separator_fields(mut self, fields: &'static [usize]) -> Self {
        self.separator_fields = fields;
        self
    }

    fn with_selector_fields(mut self, fields: &'static [usize]) -> Self {
        self.selector_fields = fields;
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

            let max_value_width = dialog.width as usize - 2 - Self::LABEL_PREFIX_WIDTH;
            let value_span = if show_placeholder {
                let ph_text = self.placeholder.unwrap().0;
                Span::styled(ph_text, Style::default().fg(Color::DarkGray))
            } else {
                let full = format!("{display_value}{cursor}");
                let visible = if full.chars().count() > max_value_width {
                    let skip = full.chars().count() - max_value_width;
                    full.chars().skip(skip).collect()
                } else {
                    full
                };
                Span::styled(visible, value_style)
            };

            lines.push(Line::from(vec![
                Span::styled(format!("  {label:<22}"), label_style),
                value_span,
            ]));
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
                        self.values[self.selected].push(c);
                    } else {
                        self.reject_flash = Some(std::time::Instant::now());
                    }
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
                if !self.hidden_fields.contains(&self.selected)
                    && !self.is_separator(self.selected)
                {
                    break;
                }
            },
            KeyCode::Down => loop {
                if self.selected >= self.labels.len() - 1 {
                    break;
                }
                self.selected += 1;
                if !self.hidden_fields.contains(&self.selected)
                    && !self.is_separator(self.selected)
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
