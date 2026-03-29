pub mod api_error;
pub mod branch;
pub mod character;
pub mod delete_confirm;
pub mod edit;
pub mod passkey;
pub mod set_passkey;
pub mod system;
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

const MULTILINE_WIDTH_PERCENT: u16 = 70;
const MULTILINE_HEIGHT_PERCENT: u16 = 60;

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
];

const SELF_FIELDS: &[&str] = &["Name", "Persona"];
const SELF_MULTILINE: &[usize] = &[1];

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

pub fn open_config_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(" Configuration ", CONFIG_FIELDS, values, &[])
}

pub fn open_self_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(" User Persona ", SELF_FIELDS, values, SELF_MULTILINE)
}

pub fn open_character_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Character ",
        CHARACTER_EDITOR_FIELDS,
        values,
        CHARACTER_EDITOR_MULTILINE,
    )
}

pub fn open_entry_editor(values: Vec<String>, selective: bool) -> FieldDialog<'static> {
    let mut dialog = FieldDialog::new(
        " Edit Entry ",
        ENTRY_EDITOR_FIELDS,
        values,
        ENTRY_EDITOR_MULTILINE,
    )
    .with_placeholder("keyword1, keyword2, ...", ENTRY_EDITOR_PLACEHOLDER_FIELDS);
    if !selective {
        dialog.hidden_fields = vec![3];
    }
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
    pub hidden_fields: Vec<usize>,
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
            hidden_fields: Vec::new(),
        }
    }

    fn with_placeholder(mut self, text: &'static str, fields: &'static [usize]) -> Self {
        self.placeholder = Some((text, fields));
        self
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
        let default_width = 60;
        let default_height = self.labels.len() as u16 + 4;
        let (w, h) = match (self.width, self.height) {
            (Some(wp), Some(hp)) => {
                let w = (area.width as f32 * wp as f32 / 100.0) as u16;
                let h = (area.height as f32 * hp as f32 / 100.0) as u16;
                (w, h)
            }
            _ => {
                let editor_extra = if self.editor.is_some() { 8 } else { 0 };
                (default_width, default_height + editor_extra)
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

            let label_style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let value_style = if is_selected {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };

            let is_empty = value.is_empty();
            let display_value = if self.is_multiline(i) && value.contains('\n') {
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
                Span::styled(format!("  {label:<15}"), label_style),
                value_span,
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Up/Down: navigate  Enter: edit  Esc: save & close",
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
                if self.is_multiline(self.selected) {
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
