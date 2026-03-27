use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::style::Style;
use tui_textarea::TextArea;

use crate::session::{self, SaveMode, Session};

use super::{Action, App};

pub fn handle_input_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.is_streaming {
        return None;
    }

    let picker_active = input_has_command_picker(app);

    if picker_active {
        let matches = crate::commands::matching_commands(
            app.textarea
                .lines()
                .join("\n")
                .split_whitespace()
                .next()
                .unwrap_or("/"),
        );
        match key.code {
            KeyCode::Up => {
                app.command_picker_selected = app.command_picker_selected.saturating_sub(1);
                return None;
            }
            KeyCode::Down => {
                app.command_picker_selected =
                    (app.command_picker_selected + 1).min(matches.len().saturating_sub(1));
                return None;
            }
            KeyCode::Tab | KeyCode::Char(' ') if !matches.is_empty() => {
                let selected = app
                    .command_picker_selected
                    .min(matches.len().saturating_sub(1));
                let cmd_name = matches[selected].name;
                let suffix = if matches[selected].args.is_empty() {
                    ""
                } else {
                    " "
                };
                app.textarea = TextArea::from(vec![format!("{cmd_name}{suffix}")]);
                app.textarea.set_cursor_line_style(Style::default());
                app.textarea
                    .move_cursor(tui_textarea::CursorMove::End);
                app.command_picker_selected = 0;
                return None;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Up if app.textarea.lines().join("").trim().is_empty() => {
            recall_last_message(app);
            None
        }
        KeyCode::Enter if !key.modifiers.contains(KeyModifiers::ALT) => {
            let lines: Vec<String> = app.textarea.lines().to_vec();
            let text = lines.join("\n");
            let trimmed = text.trim().to_owned();

            if trimmed.is_empty() {
                return None;
            }

            app.textarea = TextArea::default();
            app.textarea.set_cursor_line_style(Style::default());
            app.command_picker_selected = 0;

            if trimmed.starts_with('/') {
                let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
                let cmd = parts[0].to_owned();
                let arg = parts
                    .get(1)
                    .map(|s| s.trim().to_owned())
                    .unwrap_or_default();
                return Some(Action::SlashCommand(cmd, arg));
            }

            Some(Action::SendMessage(trimmed))
        }
        _ => {
            app.textarea.input(key);
            app.command_picker_selected = 0;
            None
        }
    }
}

fn recall_last_message(app: &mut App) {
    use crate::session::Role;

    app.session.pop_trailing_assistant();

    let user_content = app
        .session
        .tree
        .head()
        .and_then(|id| app.session.tree.node(id))
        .filter(|n| n.message.role == Role::User)
        .map(|n| n.message.content.clone());

    if let Some(content) = user_content {
        app.session.tree.pop_head();
        let lines: Vec<String> = content.lines().map(String::from).collect();
        app.textarea = TextArea::from(lines);
        app.textarea.set_cursor_line_style(Style::default());
        app.textarea.move_cursor(tui_textarea::CursorMove::Bottom);
        app.textarea.move_cursor(tui_textarea::CursorMove::End);
        app.auto_scroll = true;
    }
}

pub fn input_has_command_picker(app: &App) -> bool {
    let text = app.textarea.lines().join("\n");
    text.starts_with('/') && !text.contains(' ') && !app.is_streaming
}

pub fn handle_chat_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    match key.code {
        KeyCode::Up => {
            app.chat_scroll = app.chat_scroll.saturating_sub(1);
            app.auto_scroll = false;
            None
        }
        KeyCode::Down => {
            app.chat_scroll = app.chat_scroll.saturating_add(1);
            None
        }
        KeyCode::PageUp => {
            app.chat_scroll = app.chat_scroll.saturating_sub(10);
            app.auto_scroll = false;
            None
        }
        KeyCode::PageDown => {
            app.chat_scroll = app.chat_scroll.saturating_add(10);
            None
        }
        KeyCode::End => {
            app.auto_scroll = true;
            None
        }
        _ => None,
    }
}

pub fn handle_sidebar_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    let count = app.sidebar_sessions.len();
    if count == 0 {
        return None;
    }

    match key.code {
        KeyCode::Up => {
            let selected = app.sidebar_state.selected().unwrap_or(0);
            let new = if selected == 0 { count - 1 } else { selected - 1 };
            app.sidebar_state.select(Some(new));
            load_sidebar_selection(app);
            None
        }
        KeyCode::Down => {
            let selected = app.sidebar_state.selected().unwrap_or(0);
            let new = (selected + 1) % count;
            app.sidebar_state.select(Some(new));
            load_sidebar_selection(app);
            None
        }
        _ => None,
    }
}

fn load_sidebar_selection(app: &mut App) {
    let Some(selected) = app.sidebar_state.selected() else {
        return;
    };
    let entry = &app.sidebar_sessions[selected];
    if entry.is_new_chat {
        *app.session = Session::default();
        app.chat_scroll = 0;
        app.auto_scroll = true;
        let new_path = crate::config::sessions_dir().join(session::generate_session_name());
        app.save_mode.set_path(new_path);
        app.status_message = "New conversation started.".to_owned();
    } else {
        let path = entry.path.clone();
        let load_result = match &app.save_mode {
            SaveMode::Encrypted { key, .. } => session::load_encrypted(&path, key),
            _ => session::load(&path),
        };
        match load_result {
            Ok(loaded) => {
                *app.session = loaded;
                app.status_message = format!("Loaded: {}", entry.filename);
                app.save_mode.set_path(path);
                app.chat_scroll = 0;
                app.auto_scroll = true;
            }
            Err(e) => {
                app.status_message = format!("Error loading: {e}");
            }
        }
    }
}
