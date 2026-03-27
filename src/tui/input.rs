use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::style::Style;
use tui_textarea::TextArea;

use crate::session::{self, NodeId, Role, SaveMode, Session};

use super::{Action, App};

fn user_node_ids(app: &App) -> Vec<NodeId> {
    app.session
        .tree
        .branch_path_ids()
        .into_iter()
        .filter(|&id| {
            app.session
                .tree
                .node(id)
                .is_some_and(|n| n.message.role == Role::User)
        })
        .collect()
}

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
            navigate_up(app);
            None
        }
        KeyCode::Down
            if app.nav_cursor.is_some()
                && app.textarea.lines().join("").trim().is_empty() =>
        {
            navigate_down(app);
            None
        }
        KeyCode::Left
            if app.nav_cursor.is_some()
                && app.textarea.lines().join("").trim().is_empty() =>
        {
            switch_nav_sibling(app, -1);
            None
        }
        KeyCode::Right
            if app.nav_cursor.is_some()
                && app.textarea.lines().join("").trim().is_empty() =>
        {
            switch_nav_sibling(app, 1);
            None
        }
        KeyCode::Enter if !key.modifiers.contains(KeyModifiers::ALT) => {
            let lines: Vec<String> = app.textarea.lines().to_vec();
            let text = lines.join("\n");
            let trimmed = text.trim().to_owned();

            if trimmed.is_empty() {
                if app.nav_cursor.is_some() {
                    recall_last_message(app);
                }
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
    let target = app.nav_cursor.take();

    let (content, parent) = match target
        .and_then(|id| app.session.tree.node(id))
        .filter(|n| n.message.role == Role::User)
    {
        Some(node) => (node.message.content.clone(), node.parent),
        None => return,
    };

    app.session.tree.set_head(parent);

    let lines: Vec<String> = content.lines().map(String::from).collect();
    app.textarea = TextArea::from(lines);
    app.textarea.set_cursor_line_style(Style::default());
    app.textarea.move_cursor(tui_textarea::CursorMove::Bottom);
    app.textarea.move_cursor(tui_textarea::CursorMove::End);
    app.auto_scroll = true;
    app.status_message.clear();
}

fn switch_nav_sibling(app: &mut App, offset: isize) {
    let Some(current) = app.nav_cursor else { return };
    let siblings = app.session.tree.siblings_of(current);
    if siblings.len() <= 1 {
        return;
    }
    let Some(idx) = siblings.iter().position(|&s| s == current) else { return };
    let new_idx = (idx as isize + offset).rem_euclid(siblings.len() as isize) as usize;
    app.session.tree.switch_to(siblings[new_idx]);
    app.nav_cursor = Some(siblings[new_idx]);
    let _ = app.session.maybe_save(&app.save_mode);
}

fn navigate_up(app: &mut App) {
    let user_ids = user_node_ids(app);
    if user_ids.is_empty() {
        return;
    }

    match app.nav_cursor {
        None => {
            if let Some(&last) = user_ids.last() {
                app.nav_cursor = Some(last);
                app.auto_scroll = false;
                app.status_message =
                    "Up/Down: navigate, Left/Right: cycle branches, Enter: edit, Esc: exit"
                        .to_owned();
            }
        }
        Some(current) => {
            if let Some(pos) = user_ids.iter().position(|&id| id == current) {
                if pos > 0 {
                    app.nav_cursor = Some(user_ids[pos - 1]);
                }
            }
        }
    }
}

fn navigate_down(app: &mut App) {
    let user_ids = user_node_ids(app);
    let Some(current) = app.nav_cursor else {
        return;
    };

    if let Some(pos) = user_ids.iter().position(|&id| id == current) {
        if pos + 1 < user_ids.len() {
            app.nav_cursor = Some(user_ids[pos + 1]);
        } else {
            app.nav_cursor = None;
            app.status_message.clear();
            app.auto_scroll = true;
        }
    }
}

pub fn input_has_command_picker(app: &App) -> bool {
    let text = app.textarea.lines().join("\n");
    text.starts_with('/') && !text.contains(' ') && !app.is_streaming
}

pub fn handle_chat_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    let user_ids = user_node_ids(app);
    if user_ids.is_empty() {
        return None;
    }

    match key.code {
        KeyCode::Up => {
            if let Some(current) = app.nav_cursor {
                if let Some(pos) = user_ids.iter().position(|&id| id == current) {
                    if pos > 0 {
                        app.nav_cursor = Some(user_ids[pos - 1]);
                    }
                }
            }
            None
        }
        KeyCode::Down => {
            if let Some(current) = app.nav_cursor {
                if let Some(pos) = user_ids.iter().position(|&id| id == current) {
                    if pos + 1 < user_ids.len() {
                        app.nav_cursor = Some(user_ids[pos + 1]);
                    }
                }
            }
            None
        }
        KeyCode::Left => {
            switch_nav_sibling(app, -1);
            None
        }
        KeyCode::Right => {
            switch_nav_sibling(app, 1);
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
    app.nav_cursor = None;
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
