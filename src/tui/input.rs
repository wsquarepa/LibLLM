use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use tui_textarea::TextArea;

use crate::session::{self, Role, SaveMode, Session};

use super::{Action, App, StatusLevel};

pub fn handle_input_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.is_streaming {
        return None;
    }

    let picker_active = input_has_command_picker(app);

    if picker_active {
        let prefix = app.textarea.lines()[0].as_str();
        let hidden: &[&str] =
            if crate::config::load().debug_log { &[] } else { &["/report"] };
        let matches = crate::commands::matching_commands(
            prefix.split_whitespace().next().unwrap_or("/"),
            hidden,
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
                super::configure_textarea_at_end(&mut app.textarea);
                app.command_picker_selected = 0;
                return None;
            }
            KeyCode::Enter if !matches.is_empty() => {
                let selected = app
                    .command_picker_selected
                    .min(matches.len().saturating_sub(1));
                let cmd = matches[selected];
                if cmd.args.is_empty() {
                    app.textarea = TextArea::default();
                    super::configure_textarea(&mut app.textarea);
                    app.command_picker_selected = 0;
                    return Some(Action::SlashCommand(cmd.name.to_owned(), String::new()));
                }
                app.textarea = TextArea::from(vec![format!("{} ", cmd.name)]);
                super::configure_textarea_at_end(&mut app.textarea);
                app.command_picker_selected = 0;
                return None;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Up if textarea_is_empty(app) => {
            navigate_up(app);
            None
        }
        KeyCode::Down if app.nav_cursor.is_some() && textarea_is_empty(app) => {
            navigate_down(app);
            None
        }
        KeyCode::Left if app.nav_cursor.is_some() && textarea_is_empty(app) => {
            switch_nav_sibling(app, -1);
            None
        }
        KeyCode::Right if app.nav_cursor.is_some() && textarea_is_empty(app) => {
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
            super::configure_textarea(&mut app.textarea);
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
    super::configure_textarea_at_end(&mut app.textarea);
    app.auto_scroll = true;
}

fn switch_nav_sibling(app: &mut App, offset: isize) {
    let Some(current) = app.nav_cursor else {
        return;
    };
    let siblings = app.session.tree.siblings_of(current);
    if siblings.len() <= 1 {
        return;
    }
    let Some(idx) = siblings.iter().position(|&s| s == current) else {
        return;
    };
    let new_idx = (idx as isize + offset).rem_euclid(siblings.len() as isize) as usize;
    app.session.tree.switch_to(siblings[new_idx]);
    app.invalidate_chat_cache();
    app.nav_cursor = Some(siblings[new_idx]);
    app.mark_session_dirty(super::SaveTrigger::Debounced, false);
}

fn navigate_up(app: &mut App) {
    let next_cursor = {
        let user_ids = app.session.tree.current_user_branch_ids();
        if user_ids.is_empty() {
            return;
        }

        match app.nav_cursor {
            None => user_ids.last().copied(),
            Some(current) => user_ids
                .iter()
                .position(|&id| id == current)
                .and_then(|pos| pos.checked_sub(1))
                .map(|pos| user_ids[pos]),
        }
    };

    if app.nav_cursor.is_none() {
        if next_cursor.is_some() {
            app.auto_scroll = false;
        }
        app.nav_cursor = next_cursor;
        return;
    }

    if let Some(cursor) = next_cursor {
        app.nav_cursor = Some(cursor);
    }
}

fn navigate_down(app: &mut App) {
    let Some(current) = app.nav_cursor else {
        return;
    };

    let next_cursor = {
        let user_ids = app.session.tree.current_user_branch_ids();
        user_ids
            .iter()
            .position(|&id| id == current)
            .and_then(|pos| user_ids.get(pos + 1).copied())
    };

    if let Some(cursor) = next_cursor {
        app.nav_cursor = Some(cursor);
    } else {
        app.nav_cursor = None;
        app.auto_scroll = true;
    }
}

pub fn input_has_command_picker(app: &App) -> bool {
    if app.is_streaming {
        return false;
    }
    let lines = app.textarea.lines();
    lines.len() == 1 && lines[0].starts_with('/') && !lines[0].contains(' ')
}

fn textarea_is_empty(app: &App) -> bool {
    app.textarea.lines().iter().all(|l| l.trim().is_empty())
}

pub fn handle_chat_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.session.tree.current_branch_ids().is_empty() {
        return None;
    }

    match key.code {
        KeyCode::Up => {
            let next_cursor = {
                let all_ids = app.session.tree.current_branch_ids();
                app.nav_cursor.and_then(|current| {
                    all_ids
                        .iter()
                        .position(|&id| id == current)
                        .and_then(|pos| pos.checked_sub(1))
                        .map(|pos| all_ids[pos])
                })
            };
            if let Some(cursor) = next_cursor {
                app.nav_cursor = Some(cursor);
            }
            None
        }
        KeyCode::Down => {
            let next_cursor = {
                let all_ids = app.session.tree.current_branch_ids();
                app.nav_cursor.and_then(|current| {
                    all_ids
                        .iter()
                        .position(|&id| id == current)
                        .and_then(|pos| all_ids.get(pos + 1).copied())
                })
            };
            if let Some(cursor) = next_cursor {
                app.nav_cursor = Some(cursor);
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
        KeyCode::Enter => {
            if let Some(node_id) = app.nav_cursor {
                if let Some(node) = app.session.tree.node(node_id) {
                    let content = node.message.content.clone();
                    app.raw_edit_node = Some(node_id);
                    super::open_edit_dialog_with(app, &content);
                    app.focus = super::Focus::EditDialog;
                }
            }
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
            let new = if selected == 0 {
                count - 1
            } else {
                selected - 1
            };
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
        KeyCode::Backspace | KeyCode::Delete => {
            let selected = app.sidebar_state.selected().unwrap_or(0);
            let entry = &app.sidebar_sessions[selected];
            if entry.is_new_chat {
                return None;
            }
            app.delete_confirm_filename = entry.filename.clone();
            app.delete_confirm_selected = 0;
            app.delete_context = super::DeleteContext::Session;
            app.focus = super::Focus::DeleteConfirmDialog;
            None
        }
        _ => None,
    }
}

fn load_sidebar_selection(app: &mut App) {
    let Some(selected) = app.sidebar_state.selected() else {
        return;
    };
    if !app.flush_session_before_transition() {
        return;
    }
    app.nav_cursor = None;
    let (is_new_chat, path, filename) = {
        let entry = &app.sidebar_sessions[selected];
        (
            entry.is_new_chat,
            entry.path.clone(),
            entry.filename.clone(),
        )
    };
    if is_new_chat {
        app.discard_pending_session_save();
        let mut new_session = Session::default();
        new_session.persona = app.config.default_persona.clone();
        *app.session = new_session;
        super::business::load_active_persona(app);
        app.invalidate_chat_cache();
        app.invalidate_worldbook_cache();
        app.chat_scroll = 0;
        app.auto_scroll = true;
        let new_path = crate::config::sessions_dir().join(session::generate_session_name());
        app.save_mode.set_path(new_path);
    } else {
        let load_result = match &app.save_mode {
            SaveMode::Encrypted { key, .. } => session::load_encrypted(&path, key),
            _ => session::load(&path),
        };
        match load_result {
            Ok(loaded) => {
                app.discard_pending_session_save();
                *app.session = loaded;
                super::business::load_active_persona(app);
                app.invalidate_chat_cache();
                app.invalidate_worldbook_cache();
                app.set_status(format!("Loaded: {filename}"), super::StatusLevel::Info);
                app.save_mode.set_path(path);
                app.chat_scroll = 0;
                app.auto_scroll = true;
            }
            Err(e) => {
                app.set_status(format!("Error loading: {e}"), super::StatusLevel::Error);
            }
        }
    }
}

pub fn handle_sidebar_paste(path: &std::path::Path, ext: &str, app: &mut App) -> bool {
    if ext != "json" {
        app.set_status(
            "Session import supports .json files only.".to_owned(),
            StatusLevel::Warning,
        );
        return true;
    }

    match session::load(path) {
        Ok(loaded) => {
            let new_name = session::generate_session_name();
            let new_path = crate::config::sessions_dir().join(&new_name);

            let save_result = match &app.save_mode {
                SaveMode::Encrypted { key, .. } => {
                    session::save_encrypted(&new_path, &loaded, key)
                }
                _ => session::save(&new_path, &loaded),
            };

            match save_result {
                Ok(()) => {
                    let count = loaded.tree.current_branch_ids().len();
                    super::business::refresh_sidebar(app);
                    app.set_status(
                        format!("Imported session ({count} messages)."),
                        StatusLevel::Info,
                    );
                }
                Err(e) => {
                    app.set_status(format!("Save error: {e}"), StatusLevel::Error);
                }
            }
        }
        Err(e) => {
            app.set_status(format!("Import error: {e}"), StatusLevel::Error);
        }
    }
    true
}
