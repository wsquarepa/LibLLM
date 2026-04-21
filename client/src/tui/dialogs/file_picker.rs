//! Shell-path file picker: opened when the user types `@` at a word
//! boundary in the input. The picker lists directory entries starting
//! from the current CWD and narrows as the user types. Press Enter or
//! Tab on a directory to descend into it, on a file to accept it. `~`,
//! `..`, and absolute paths are supported. Accepting an entry replaces
//! the typed `@<prefix>` with `@<full-path>`.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::text::Span;

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::tui::{Action, App, Focus};

pub(in crate::tui) const FILE_PICKER_DIALOG_WIDTH: u16 = 80;
pub(in crate::tui) const FILE_PICKER_DIALOG_HEIGHT: u16 = 20;

fn sanitize_display_name(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_control() { '\u{FFFD}' } else { c })
        .collect()
}

#[derive(Debug, Clone)]
pub struct FilePickerEntry {
    pub name: String,
    pub is_dir: bool,
}

/// State for an open file picker.
///
/// - `base_dir` is the directory whose entries are being listed.
/// - `filter` is the per-entry prefix that the user has typed since the
///   last `/`.
/// - `anchor_line` / `anchor_col` point at the byte offset of the `@`
///   in the textarea; used to replace the token on accept.
pub struct FilePickerState {
    pub base_dir: PathBuf,
    pub filter: String,
    pub entries: Vec<FilePickerEntry>,
    pub selected: usize,
    pub list_state: ListState,
    pub anchor_line: usize,
    pub anchor_col: usize,
}

impl FilePickerState {
    pub fn new(base_dir: PathBuf, anchor_line: usize, anchor_col: usize) -> Self {
        let mut state = Self {
            base_dir,
            filter: String::new(),
            entries: Vec::new(),
            selected: 0,
            list_state: ListState::default(),
            anchor_line,
            anchor_col,
        };
        state.refresh_entries();
        state.list_state.select(Some(0));
        state
    }

    pub fn refresh_entries(&mut self) {
        self.entries = match std::fs::read_dir(&self.base_dir) {
            Ok(rd) => rd
                .filter_map(|r| r.ok())
                .map(|d| {
                    let is_dir = d.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    FilePickerEntry {
                        name: d.file_name().to_string_lossy().into_owned(),
                        is_dir,
                    }
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        self.entries.sort_by(|a, b| {
            a.is_dir
                .cmp(&b.is_dir)
                .reverse()
                .then_with(|| a.name.cmp(&b.name))
        });
        self.selected = 0;
        self.list_state.select(Some(0));
    }

    pub fn visible(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.name.starts_with(&self.filter))
            .map(|(i, _)| i)
            .collect()
    }

    /// Selected entry, or None if the filtered list is empty.
    pub fn current(&self) -> Option<&FilePickerEntry> {
        self.visible()
            .get(self.selected)
            .and_then(|&i| self.entries.get(i))
    }

    /// Descend into the currently selected directory entry. Clears the
    /// typed filter and refreshes the listing to the new directory.
    /// Returns `true` when the descent happened; returns `false` when
    /// there is no selected entry or when the selection is a file.
    pub fn descend(&mut self) -> bool {
        let Some(entry) = self.current().cloned() else {
            return false;
        };
        if !entry.is_dir {
            return false;
        }
        self.base_dir.push(&entry.name);
        self.filter.clear();
        self.refresh_entries();
        true
    }
}

/// Expand a user-typed path fragment into a (base_dir, filter) pair.
/// Handles `~`, `~/`, `..`, absolute, and relative paths. `cwd` is the
/// process CWD, used for relative resolution.
pub fn split_fragment(cwd: &Path, typed: &str) -> (PathBuf, String) {
    if typed.is_empty() {
        return (cwd.to_path_buf(), String::new());
    }
    let expanded = if let Some(rest) = typed.strip_prefix("~/") {
        dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| PathBuf::from(typed))
    } else if typed == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(typed))
    } else if Path::new(typed).is_absolute() {
        PathBuf::from(typed)
    } else {
        cwd.join(typed)
    };
    if typed.ends_with('/') {
        (expanded, String::new())
    } else {
        let parent = expanded
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| cwd.to_path_buf());
        let file = expanded
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();
        (parent, file)
    }
}

pub(in crate::tui) fn render(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let dialog = clear_centered(f, FILE_PICKER_DIALOG_WIDTH, FILE_PICKER_DIALOG_HEIGHT, area);

    let Some(state) = app.file_picker.as_mut() else {
        return;
    };

    let title = format!(
        " File @ {} ",
        sanitize_display_name(&state.base_dir.to_string_lossy())
    );
    let block = dialog_block(title, app.theme.border_focused);

    let visible: Vec<usize> = state
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.name.starts_with(&state.filter))
        .map(|(i, _)| i)
        .collect();

    let items: Vec<ListItem<'_>> = visible
        .iter()
        .map(|&i| {
            let entry = &state.entries[i];
            let safe = sanitize_display_name(&entry.name);
            let label = if entry.is_dir {
                format!("{safe}/")
            } else {
                safe
            };
            let style = if entry.is_dir {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Span::styled(label, style))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(app.theme.nav_cursor_bg)
                .fg(app.theme.nav_cursor_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    f.render_stateful_widget(list, dialog, &mut state.list_state);

    if visible.is_empty() {
        let empty = Paragraph::new("(no matching entries)")
            .alignment(Alignment::Center)
            .style(Style::default().fg(app.theme.dimmed));
        let inner = Rect {
            x: dialog.x + 2,
            y: dialog.y + (dialog.height / 2),
            width: dialog.width.saturating_sub(4),
            height: 1,
        };
        f.render_widget(empty, inner);
    }

    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[Line::from(
            "Up/Down: move  Enter/Tab: open/accept  Backspace: parent  Esc: close",
        )],
    );
}

pub(in crate::tui) fn handle_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    let Some(state) = app.file_picker.as_mut() else {
        app.focus = Focus::Input;
        return None;
    };

    match key.code {
        KeyCode::Up => {
            let visible_len = state.visible().len();
            if visible_len > 0 {
                state.selected = state.selected.saturating_sub(1);
                state.list_state.select(Some(state.selected));
            }
            None
        }
        KeyCode::Down => {
            let visible_len = state.visible().len();
            if visible_len > 0 && state.selected + 1 < visible_len {
                state.selected += 1;
                state.list_state.select(Some(state.selected));
            }
            None
        }
        KeyCode::Enter | KeyCode::Tab => {
            if !state.descend() {
                accept_current(app);
            }
            None
        }
        KeyCode::Char(' ') | KeyCode::Esc => {
            app.file_picker = None;
            app.focus = Focus::Input;
            None
        }
        KeyCode::Backspace => {
            if !state.filter.is_empty() {
                state.filter.pop();
                state.selected = 0;
                state.list_state.select(Some(0));
            } else if let Some(parent) = state.base_dir.parent().map(Path::to_path_buf) {
                state.base_dir = parent;
                state.refresh_entries();
            }
            None
        }
        KeyCode::Char(c) if !c.is_control() && c != '/' => {
            state.filter.push(c);
            state.selected = 0;
            state.list_state.select(Some(0));
            None
        }
        _ => None,
    }
}

fn accept_current(app: &mut App) {
    let (full_path, anchor_line, anchor_col, filter_len) = {
        let Some(state) = app.file_picker.as_ref() else {
            return;
        };
        let Some(entry) = state.current() else {
            return;
        };
        let mut full = state.base_dir.clone();
        full.push(&entry.name);
        (
            full.to_string_lossy().into_owned(),
            state.anchor_line,
            state.anchor_col,
            state.filter.len(),
        )
    };

    let Some(replacement) = crate::tui::events::format_at_token(&full_path) else {
        app.set_status(
            format!("Cannot attach file: path contains quote character: {full_path}"),
            crate::tui::StatusLevel::Warning,
        );
        app.file_picker = None;
        app.focus = Focus::Input;
        return;
    };

    let textarea = &mut app.textarea;
    let mut lines: Vec<String> = textarea.lines().to_vec();
    if anchor_line >= lines.len() {
        app.file_picker = None;
        app.focus = Focus::Input;
        return;
    }
    let line = &mut lines[anchor_line];
    let token_end = (anchor_col + 1 + filter_len).min(line.len());
    let head = line[..anchor_col].to_owned();
    let tail = line[token_end..].to_owned();
    let new_line = format!("{head}{replacement}{tail}");
    let new_cursor_col = anchor_col + replacement.len();
    *line = new_line;
    *textarea = tui_textarea::TextArea::from(lines);
    super::super::dialog_handler::configure_textarea(textarea);
    textarea.move_cursor(tui_textarea::CursorMove::Jump(
        anchor_line as u16,
        new_cursor_col as u16,
    ));

    app.file_picker = None;
    app.focus = Focus::Input;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn sanitize_display_name_replaces_control_chars() {
        let raw = "safe\x1b[31mBAD\x1b[0m\x07\x7f";
        let out = sanitize_display_name(raw);
        assert!(!out.contains('\x1b'));
        assert!(!out.contains('\x07'));
        assert!(!out.contains('\x7f'));
        assert!(out.contains("safe"));
        assert!(out.contains("BAD"));
    }

    #[test]
    fn sanitize_display_name_preserves_printable_utf8() {
        let raw = "日記.md";
        assert_eq!(sanitize_display_name(raw), raw);
    }

    #[test]
    fn split_fragment_empty_keeps_cwd_empty_filter() {
        let tmp = TempDir::new().unwrap();
        let (dir, filter) = split_fragment(tmp.path(), "");
        assert_eq!(dir, tmp.path());
        assert_eq!(filter, "");
    }

    #[test]
    fn split_fragment_prefix_sets_filter() {
        let tmp = TempDir::new().unwrap();
        let (dir, filter) = split_fragment(tmp.path(), "no");
        assert_eq!(dir, tmp.path());
        assert_eq!(filter, "no");
    }

    #[test]
    fn split_fragment_trailing_slash_enters_dir() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let (dir, filter) = split_fragment(tmp.path(), "sub/");
        assert_eq!(dir, tmp.path().join("sub"));
        assert_eq!(filter, "");
    }

    #[test]
    fn refresh_entries_sorts_dirs_first() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("zdir")).unwrap();
        std::fs::write(tmp.path().join("afile.md"), "x").unwrap();
        let state = FilePickerState::new(tmp.path().to_path_buf(), 0, 0);
        assert_eq!(state.entries.len(), 2);
        assert_eq!(state.entries[0].name, "zdir");
        assert!(state.entries[0].is_dir);
        assert_eq!(state.entries[1].name, "afile.md");
    }

    #[test]
    fn visible_filters_by_prefix() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("apple"), "x").unwrap();
        std::fs::write(tmp.path().join("apricot"), "x").unwrap();
        std::fs::write(tmp.path().join("banana"), "x").unwrap();
        let mut state = FilePickerState::new(tmp.path().to_path_buf(), 0, 0);
        state.filter = "ap".to_owned();
        let visible: Vec<&str> = state
            .visible()
            .into_iter()
            .map(|i| state.entries[i].name.as_str())
            .collect();
        assert_eq!(visible, vec!["apple", "apricot"]);
    }

    #[test]
    fn descend_enters_selected_directory_and_refreshes() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub").join("nested.txt"), "x").unwrap();
        let mut state = FilePickerState::new(tmp.path().to_path_buf(), 0, 0);
        assert!(state.descend());
        assert_eq!(state.base_dir, tmp.path().join("sub"));
        assert_eq!(state.filter, "");
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].name, "nested.txt");
    }

    #[test]
    fn descend_on_file_returns_false_and_keeps_state() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("only.txt"), "x").unwrap();
        let mut state = FilePickerState::new(tmp.path().to_path_buf(), 0, 0);
        let base_before = state.base_dir.clone();
        assert!(!state.descend());
        assert_eq!(state.base_dir, base_before);
    }

    #[test]
    fn descend_with_empty_entries_returns_false() {
        let tmp = TempDir::new().unwrap();
        let mut state = FilePickerState::new(tmp.path().to_path_buf(), 0, 0);
        assert!(!state.descend());
    }
}
