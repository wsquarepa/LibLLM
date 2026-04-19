//! Authentication sub-dialog: type + per-variant field editor for `/config`.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{LIST_DIALOG_TALL_PADDING, LIST_DIALOG_WIDTH};
use super::{byte_pos_at_char, clear_centered, dialog_block, render_hints_below_dialog};
use crate::tui::types::{App, Focus, StatusLevel};
use libllm::config::{Auth, AuthKind, AuthOverrides};

const AUTH_DIALOG_WIDTH: u16 = 60;
const AUTH_DIALOG_PADDING: u16 = 6;

const TYPE_PICKER_OPTIONS: &[AuthKind] = &[
    AuthKind::None,
    AuthKind::Basic,
    AuthKind::Bearer,
    AuthKind::Header,
    AuthKind::Query,
];

pub(in crate::tui) enum AuthDialogAction {
    Continue,
    Close,
    OpenTypePicker,
}

struct AuthScratchpad {
    basic_username: String,
    basic_password: String,
    bearer_token: String,
    header_name: String,
    header_value: String,
    query_name: String,
    query_value: String,
}

impl AuthScratchpad {
    fn from_auth(auth: &Auth) -> Self {
        Self {
            basic_username: auth.basic_username(),
            basic_password: auth.basic_password(),
            bearer_token: auth.bearer_token(),
            header_name: auth.header_name(),
            header_value: auth.header_value(),
            query_name: auth.query_name(),
            query_value: auth.query_value(),
        }
    }
}

#[derive(Clone, Copy)]
enum FieldSlot {
    Type,
    BasicUsername,
    BasicPassword,
    BearerToken,
    HeaderName,
    HeaderValue,
    QueryName,
    QueryValue,
}

impl FieldSlot {
    fn is_secret(self) -> bool {
        matches!(
            self,
            FieldSlot::BasicPassword
                | FieldSlot::BearerToken
                | FieldSlot::HeaderValue
                | FieldSlot::QueryValue
        )
    }
}

struct LocksBySlot {
    auth_type: bool,
    basic_username: bool,
    basic_password: bool,
    bearer_token: bool,
    header_name: bool,
    header_value: bool,
    query_name: bool,
    query_value: bool,
}

impl LocksBySlot {
    fn from_overrides(overrides: &AuthOverrides) -> Self {
        Self {
            auth_type: overrides.auth_type.is_some(),
            basic_username: overrides.auth_basic_username.is_some(),
            basic_password: overrides.auth_basic_password.is_some(),
            bearer_token: overrides.auth_bearer_token.is_some(),
            header_name: overrides.auth_header_name.is_some(),
            header_value: overrides.auth_header_value.is_some(),
            query_name: overrides.auth_query_name.is_some(),
            query_value: overrides.auth_query_value.is_some(),
        }
    }

    fn is_locked(&self, slot: FieldSlot) -> bool {
        match slot {
            FieldSlot::Type => self.auth_type,
            FieldSlot::BasicUsername => self.basic_username,
            FieldSlot::BasicPassword => self.basic_password,
            FieldSlot::BearerToken => self.bearer_token,
            FieldSlot::HeaderName => self.header_name,
            FieldSlot::HeaderValue => self.header_value,
            FieldSlot::QueryName => self.query_name,
            FieldSlot::QueryValue => self.query_value,
        }
    }
}

pub(in crate::tui) struct AuthDialogState {
    active_type: AuthKind,
    scratchpad: AuthScratchpad,
    labels: Vec<&'static str>,
    slots: Vec<FieldSlot>,
    selected: usize,
    editing: bool,
    cursor_pos: usize,
    locked: Vec<bool>,
    value_changed: bool,
    type_picker_selected: usize,
}

fn rows_for(kind: AuthKind) -> (Vec<&'static str>, Vec<FieldSlot>) {
    match kind {
        AuthKind::None => (vec!["Type"], vec![FieldSlot::Type]),
        AuthKind::Basic => (
            vec!["Type", "Username", "Password"],
            vec![
                FieldSlot::Type,
                FieldSlot::BasicUsername,
                FieldSlot::BasicPassword,
            ],
        ),
        AuthKind::Bearer => (
            vec!["Type", "Token"],
            vec![FieldSlot::Type, FieldSlot::BearerToken],
        ),
        AuthKind::Header => (
            vec!["Type", "Name", "Value"],
            vec![
                FieldSlot::Type,
                FieldSlot::HeaderName,
                FieldSlot::HeaderValue,
            ],
        ),
        AuthKind::Query => (
            vec!["Type", "Name", "Value"],
            vec![FieldSlot::Type, FieldSlot::QueryName, FieldSlot::QueryValue],
        ),
    }
}

impl AuthDialogState {
    fn from_auth(auth: &Auth, locks: &LocksBySlot) -> Self {
        let active_type = auth.kind();
        let (labels, slots) = rows_for(active_type);
        let locked = slots.iter().map(|s| locks.is_locked(*s)).collect();
        Self {
            active_type,
            scratchpad: AuthScratchpad::from_auth(auth),
            labels,
            slots,
            selected: 0,
            editing: false,
            cursor_pos: 0,
            locked,
            value_changed: false,
            type_picker_selected: 0,
        }
    }

    fn rebuild_for_type(&mut self, new_type: AuthKind, locks: &LocksBySlot) {
        let (labels, slots) = rows_for(new_type);
        self.active_type = new_type;
        self.labels = labels;
        self.slots = slots;
        self.locked = self.slots.iter().map(|s| locks.is_locked(*s)).collect();
        self.selected = 0;
        self.editing = false;
        self.cursor_pos = 0;
    }

    fn slot_value(&self, slot: FieldSlot) -> &str {
        match slot {
            FieldSlot::Type => auth_kind_label(self.active_type),
            FieldSlot::BasicUsername => &self.scratchpad.basic_username,
            FieldSlot::BasicPassword => &self.scratchpad.basic_password,
            FieldSlot::BearerToken => &self.scratchpad.bearer_token,
            FieldSlot::HeaderName => &self.scratchpad.header_name,
            FieldSlot::HeaderValue => &self.scratchpad.header_value,
            FieldSlot::QueryName => &self.scratchpad.query_name,
            FieldSlot::QueryValue => &self.scratchpad.query_value,
        }
    }

    fn slot_value_mut(&mut self, slot: FieldSlot) -> Option<&mut String> {
        match slot {
            FieldSlot::Type => None,
            FieldSlot::BasicUsername => Some(&mut self.scratchpad.basic_username),
            FieldSlot::BasicPassword => Some(&mut self.scratchpad.basic_password),
            FieldSlot::BearerToken => Some(&mut self.scratchpad.bearer_token),
            FieldSlot::HeaderName => Some(&mut self.scratchpad.header_name),
            FieldSlot::HeaderValue => Some(&mut self.scratchpad.header_value),
            FieldSlot::QueryName => Some(&mut self.scratchpad.query_name),
            FieldSlot::QueryValue => Some(&mut self.scratchpad.query_value),
        }
    }

    fn build_candidate_auth(&self) -> Auth {
        match self.active_type {
            AuthKind::None => Auth::None,
            AuthKind::Basic => Auth::Basic {
                username: self.scratchpad.basic_username.clone(),
                password: self.scratchpad.basic_password.clone(),
            },
            AuthKind::Bearer => Auth::Bearer {
                token: self.scratchpad.bearer_token.clone(),
            },
            AuthKind::Header => Auth::Header {
                name: self.scratchpad.header_name.clone(),
                value: self.scratchpad.header_value.clone(),
            },
            AuthKind::Query => Auth::Query {
                name: self.scratchpad.query_name.clone(),
                value: self.scratchpad.query_value.clone(),
            },
        }
    }
}

fn auth_kind_label(kind: AuthKind) -> &'static str {
    match kind {
        AuthKind::None => "None",
        AuthKind::Basic => "Basic",
        AuthKind::Bearer => "Bearer",
        AuthKind::Header => "Header",
        AuthKind::Query => "Query",
    }
}

pub(in crate::tui) fn open_auth_dialog(app: &mut App) {
    let locks = LocksBySlot::from_overrides(&app.cli_overrides.auth_overrides());
    let state = AuthDialogState::from_auth(&app.config.auth, &locks);
    app.auth_dialog = Some(state);
    app.focus = Focus::AuthDialog;
}

pub(in crate::tui) fn render_auth_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let Some(state) = app.auth_dialog.as_ref() else {
        return;
    };
    let height = (state.labels.len() as u16) + AUTH_DIALOG_PADDING;
    let dialog = clear_centered(f, AUTH_DIALOG_WIDTH, height, area);

    let mut lines: Vec<Line> = vec![Line::from("")];
    for (i, label) in state.labels.iter().enumerate() {
        let slot = state.slots[i];
        let is_selected = i == state.selected;
        let is_locked = state.locked[i];
        let raw = state.slot_value(slot);
        let display_value: String = if slot.is_secret() {
            "*".repeat(raw.chars().count())
        } else {
            raw.to_owned()
        };

        let style = if is_locked {
            Style::default().fg(Color::Red)
        } else if is_selected && state.editing {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let marker = if is_selected { ">" } else { " " };
        if is_selected && state.editing {
            let chars: Vec<char> = display_value.chars().collect();
            let cursor_pos = state.cursor_pos.min(chars.len());
            let prefix: String = chars[..cursor_pos].iter().collect();
            let at_cursor: String = if cursor_pos < chars.len() {
                chars[cursor_pos].to_string()
            } else {
                " ".to_string()
            };
            let suffix: String = if cursor_pos < chars.len() {
                chars[cursor_pos + 1..].iter().collect()
            } else {
                String::new()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{marker} {label}: "), style),
                Span::styled(prefix, style),
                Span::styled(at_cursor, style.add_modifier(Modifier::REVERSED)),
                Span::styled(suffix, style),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!("{marker} {label}: "), style),
                Span::styled(display_value, style),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines).block(dialog_block(" Authentication ", Color::Yellow));
    f.render_widget(paragraph, dialog);

    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[Line::from(
            "Up/Down: navigate  Enter: edit/select  Esc: close",
        )],
    );
}

pub(in crate::tui) fn handle_auth_dialog_key(key: KeyEvent, app: &mut App) -> AuthDialogAction {
    let Some(state) = app.auth_dialog.as_mut() else {
        return AuthDialogAction::Close;
    };

    if state.editing {
        return handle_editing_key(state, key);
    }

    match key.code {
        KeyCode::Up => {
            if state.selected > 0 {
                state.selected -= 1;
            }
            AuthDialogAction::Continue
        }
        KeyCode::Down => {
            if state.selected + 1 < state.labels.len() {
                state.selected += 1;
            }
            AuthDialogAction::Continue
        }
        KeyCode::Enter => {
            if state.locked[state.selected] {
                return AuthDialogAction::Continue;
            }
            let slot = state.slots[state.selected];
            if matches!(slot, FieldSlot::Type) {
                AuthDialogAction::OpenTypePicker
            } else {
                state.editing = true;
                state.cursor_pos = state.slot_value(slot).chars().count();
                AuthDialogAction::Continue
            }
        }
        KeyCode::Esc => AuthDialogAction::Close,
        _ => AuthDialogAction::Continue,
    }
}

fn handle_editing_key(state: &mut AuthDialogState, key: KeyEvent) -> AuthDialogAction {
    let slot = state.slots[state.selected];
    match key.code {
        KeyCode::Enter | KeyCode::Esc => {
            state.editing = false;
            AuthDialogAction::Continue
        }
        KeyCode::Char(c) => {
            let cursor = state.cursor_pos;
            if let Some(buf) = state.slot_value_mut(slot) {
                let byte_pos = byte_pos_at_char(buf, cursor);
                buf.insert(byte_pos, c);
                state.cursor_pos = cursor + 1;
                state.value_changed = true;
            }
            AuthDialogAction::Continue
        }
        KeyCode::Backspace => {
            if state.cursor_pos > 0 {
                let cursor = state.cursor_pos - 1;
                if let Some(buf) = state.slot_value_mut(slot) {
                    let byte_pos = byte_pos_at_char(buf, cursor);
                    buf.remove(byte_pos);
                    state.cursor_pos = cursor;
                    state.value_changed = true;
                }
            }
            AuthDialogAction::Continue
        }
        KeyCode::Delete => {
            let cursor = state.cursor_pos;
            if let Some(buf) = state.slot_value_mut(slot) {
                let char_count = buf.chars().count();
                if cursor < char_count {
                    let byte_pos = byte_pos_at_char(buf, cursor);
                    buf.remove(byte_pos);
                    state.value_changed = true;
                }
            }
            AuthDialogAction::Continue
        }
        KeyCode::Left => {
            if state.cursor_pos > 0 {
                state.cursor_pos -= 1;
            }
            AuthDialogAction::Continue
        }
        KeyCode::Right => {
            let char_count = state.slot_value(slot).chars().count();
            if state.cursor_pos < char_count {
                state.cursor_pos += 1;
            }
            AuthDialogAction::Continue
        }
        KeyCode::Home => {
            state.cursor_pos = 0;
            AuthDialogAction::Continue
        }
        KeyCode::End => {
            state.cursor_pos = state.slot_value(slot).chars().count();
            AuthDialogAction::Continue
        }
        _ => AuthDialogAction::Continue,
    }
}

pub(in crate::tui) fn close_and_persist(app: &mut App) {
    let Some(state) = app.auth_dialog.take() else {
        app.focus = Focus::ConfigDialog;
        return;
    };
    if !state.value_changed {
        app.focus = Focus::ConfigDialog;
        return;
    }
    let candidate = state.build_candidate_auth();
    if let Err(e) = candidate.validate() {
        app.set_status(format!("Auth: {e}"), StatusLevel::Error);
        app.auth_dialog = Some(state);
        return;
    }
    let mut cfg = app.config.clone();
    cfg.auth = candidate;
    if let Err(e) = libllm::config::save(&cfg) {
        app.set_status(format!("Failed to save auth: {e}"), StatusLevel::Error);
        app.auth_dialog = Some(state);
        return;
    }
    crate::tui::business::apply_config(app);
    if let Some(dialog) = app.config_dialog.as_mut() {
        dialog.set_value(0, 1, app.config.auth.display_label().to_owned());
    }
    app.focus = Focus::ConfigDialog;
}

pub(in crate::tui) fn open_type_picker(app: &mut App) {
    let Some(state) = app.auth_dialog.as_mut() else {
        return;
    };
    state.type_picker_selected = TYPE_PICKER_OPTIONS
        .iter()
        .position(|k| *k == state.active_type)
        .unwrap_or(0);
    app.focus = Focus::AuthTypePicker;
}

pub(in crate::tui) fn render_type_picker(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let Some(state) = app.auth_dialog.as_ref() else {
        return;
    };
    let count = TYPE_PICKER_OPTIONS.len() as u16;
    let dialog = clear_centered(f, LIST_DIALOG_WIDTH, count + LIST_DIALOG_TALL_PADDING, area);

    let mut lines: Vec<Line> = vec![Line::from("")];
    for (i, kind) in TYPE_PICKER_OPTIONS.iter().enumerate() {
        let is_selected = i == state.type_picker_selected;
        let marker = if is_selected { "> " } else { "  " };
        let style = if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{}", auth_kind_label(*kind)),
            style,
        )));
    }

    let paragraph = Paragraph::new(lines).block(dialog_block(" Select Auth Type ", Color::Yellow));
    f.render_widget(paragraph, dialog);

    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[Line::from("Up/Down: navigate  Enter: select  Esc: cancel")],
    );
}

pub(in crate::tui) fn handle_type_picker_key(
    key: KeyEvent,
    app: &mut App,
) -> Option<crate::tui::types::Action> {
    let Some(state) = app.auth_dialog.as_mut() else {
        app.focus = Focus::ConfigDialog;
        return None;
    };
    match key.code {
        KeyCode::Up if state.type_picker_selected > 0 => {
            state.type_picker_selected -= 1;
        }
        KeyCode::Down if state.type_picker_selected + 1 < TYPE_PICKER_OPTIONS.len() => {
            state.type_picker_selected += 1;
        }
        KeyCode::Enter => {
            if state.locked.first().copied().unwrap_or(false) {
                app.focus = Focus::AuthDialog;
                return None;
            }
            let chosen = TYPE_PICKER_OPTIONS[state.type_picker_selected];
            if chosen != state.active_type {
                let locks = LocksBySlot::from_overrides(&app.cli_overrides.auth_overrides());
                state.rebuild_for_type(chosen, &locks);
                state.value_changed = true;
            }
            app.focus = Focus::AuthDialog;
        }
        KeyCode::Esc => {
            app.focus = Focus::AuthDialog;
        }
        _ => {}
    }
    None
}
