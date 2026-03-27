pub mod branch;
pub mod character;
pub mod delete_confirm;
pub mod edit;
pub mod passkey;
pub mod system;
pub mod worldbook;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use tui_textarea::TextArea;

use super::render::centered_rect;

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
}

impl<'a> FieldDialog<'a> {
    pub fn new(
        title: &'static str,
        labels: &'static [&'static str],
        values: Vec<String>,
        multiline_fields: &'static [usize],
    ) -> Self {
        Self {
            title,
            labels,
            values,
            selected: 0,
            editing: false,
            multiline_fields,
            editor: None,
            width: None,
            height: None,
        }
    }

    pub fn with_size(mut self, width: u16, height: u16) -> Self {
        self.width = Some(width);
        self.height = Some(height);
        self
    }

    fn is_multiline(&self, index: usize) -> bool {
        self.multiline_fields.contains(&index)
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
        let dialog = centered_rect(w, h, area);
        f.render_widget(ratatui::widgets::Clear, dialog);

        if self.editor.is_some() {
            self.render_with_editor(f, dialog);
        } else {
            self.render_fields(f, dialog);
        }
    }

    fn render_fields(&self, f: &mut ratatui::Frame, dialog: Rect) {
        let mut lines: Vec<Line> = vec![Line::from("")];

        for (i, &label) in self.labels.iter().enumerate() {
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

            let display_value = if self.is_multiline(i) && value.contains('\n') {
                format!("({} lines)", value.lines().count())
            } else {
                value.clone()
            };

            lines.push(Line::from(vec![
                Span::styled(format!("  {label:<15}"), label_style),
                Span::styled(format!("{display_value}{cursor}"), value_style),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Up/Down: navigate  Enter: edit  Esc: save & close",
            Style::default().fg(Color::DarkGray),
        )));

        let paragraph = Paragraph::new(Text::from(lines)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(self.title)
                .border_style(Style::default().fg(Color::Yellow)),
        );

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
        let header_area = Rect {
            height: 2,
            ..inner
        };
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

        let border = Block::default()
            .borders(Borders::ALL)
            .title(self.title)
            .border_style(Style::default().fg(Color::Yellow));
        f.render_widget(border, dialog);
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
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(self.labels.len() - 1);
            }
            KeyCode::Enter => {
                if self.is_multiline(self.selected) {
                    let content = &self.values[self.selected];
                    let lines: Vec<String> =
                        content.lines().map(String::from).collect();
                    let mut editor = TextArea::from(if lines.is_empty() {
                        vec![String::new()]
                    } else {
                        lines
                    });
                    super::configure_textarea_at_end(&mut editor);
                    self.editor = Some(editor);
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
