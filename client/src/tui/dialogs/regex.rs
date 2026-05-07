//! Manage regex find/replace rules: list view, edit form, save flow.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::ListItem;

use libllm::regex_rules::{RegexRule, Scope, Target};

use super::{clear_centered, render_hints_below_dialog};
use crate::tui::dialog_handler::return_to_input;
use crate::tui::{Action, App};

pub struct RegexEditorState {
    pub original_index: Option<usize>,
    pub draft: RegexRule,
    pub sample_input: String,
    pub field: EditorField,
    pub error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditorField {
    Name,
    Pattern,
    Replacement,
    ScopeToggles,
    TargetToggles,
    Enabled,
    SampleInput,
}

impl EditorField {
    #[expect(dead_code, reason = "used by T10 editor tab-navigation")]
    fn next(self) -> Self {
        match self {
            Self::Name => Self::Pattern,
            Self::Pattern => Self::Replacement,
            Self::Replacement => Self::ScopeToggles,
            Self::ScopeToggles => Self::TargetToggles,
            Self::TargetToggles => Self::Enabled,
            Self::Enabled => Self::SampleInput,
            Self::SampleInput => Self::Name,
        }
    }
}

pub(in crate::tui) fn open(app: &mut App) {
    app.regex_list_selected = app
        .regex_list_selected
        .min(app.config.regex.len().saturating_sub(1));
    app.regex_editor = None;
    app.focus = crate::tui::Focus::RegexDialog;
}

pub(in crate::tui) fn render_regex_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let labels: Vec<String> = app
        .config
        .regex
        .iter()
        .map(format_rule_summary)
        .collect();
    let count = labels.len();
    let height = super::paged_list_height(count, area.height, super::FIELD_DIALOG_PADDING_ROWS);
    let dialog = clear_centered(f, super::LIST_DIALOG_WIDTH, height, area);
    let items: Vec<ListItem<'_>> = labels.iter().cloned().map(ListItem::new).collect();

    super::render_paged_list(
        f,
        dialog,
        &app.theme,
        super::PagedListContent {
            selected: app.regex_list_selected,
            items,
            title_base: " Regex rules ",
            search: None,
            unfiltered_total: None,
        },
    );

    let hints = vec![
        Line::from("Up/Down: navigate  Space: toggle  Enter: edit  n: new  d: delete"),
        Line::from("Shift+Up/Down: reorder  Esc: close"),
    ];
    render_hints_below_dialog(f, dialog, area, &hints);
}

fn format_rule_summary(rule: &RegexRule) -> String {
    let enabled = if rule.enabled { "[x]" } else { "[ ]" };
    let scopes: String = rule
        .scope
        .iter()
        .map(|s| match s {
            Scope::Display => 'D',
            Scope::PromptSend => 'S',
            Scope::PromptRecv => 'R',
            Scope::Export => 'E',
        })
        .collect();
    let targets: String = rule
        .target
        .iter()
        .map(|t| match t {
            Target::User => "user",
            Target::Assistant => "asst",
            Target::System => "sys",
            Target::Summary => "sum",
        })
        .collect::<Vec<_>>()
        .join("/");
    let warn = if rule.compile_error.is_some() {
        "  WARN invalid"
    } else {
        ""
    };
    format!(
        "{enabled} {name:<32}  {scopes:<4}  {targets}{warn}",
        name = rule.name
    )
}

pub(in crate::tui) fn handle_regex_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.regex_editor.is_some() {
        return handle_editor_key(key, app);
    }
    handle_list_key(key, app)
}

fn handle_list_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    let len = app.config.regex.len();
    match key.code {
        KeyCode::Esc => {
            return_to_input(app);
        }
        KeyCode::Up
            if !key.modifiers.contains(KeyModifiers::SHIFT) && app.regex_list_selected > 0 =>
        {
            app.regex_list_selected -= 1;
        }
        KeyCode::Down
            if !key.modifiers.contains(KeyModifiers::SHIFT)
                && app.regex_list_selected + 1 < len =>
        {
            app.regex_list_selected += 1;
        }
        KeyCode::Up
            if key.modifiers.contains(KeyModifiers::SHIFT)
                && app.regex_list_selected > 0
                && len >= 2 =>
        {
            let i = app.regex_list_selected;
            app.config.regex.swap(i, i - 1);
            app.regex_list_selected -= 1;
            save_and_recompile(app);
        }
        KeyCode::Down
            if key.modifiers.contains(KeyModifiers::SHIFT)
                && app.regex_list_selected + 1 < len =>
        {
            let i = app.regex_list_selected;
            app.config.regex.swap(i, i + 1);
            app.regex_list_selected += 1;
            save_and_recompile(app);
        }
        KeyCode::Char(' ') if len > 0 => {
            let i = app.regex_list_selected;
            app.config.regex[i].enabled = !app.config.regex[i].enabled;
            save_and_recompile(app);
        }
        KeyCode::Enter if len > 0 => {
            open_editor_for_existing(app, app.regex_list_selected);
        }
        KeyCode::Char('n') => {
            open_editor_for_new(app);
        }
        KeyCode::Char('d') if len > 0 => {
            let i = app.regex_list_selected;
            app.config.regex.remove(i);
            if app.regex_list_selected >= app.config.regex.len()
                && app.regex_list_selected > 0
            {
                app.regex_list_selected -= 1;
            }
            save_and_recompile(app);
        }
        _ => {}
    }
    None
}

fn handle_editor_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if key.code == KeyCode::Esc {
        app.regex_editor = None;
    }
    None
}

fn open_editor_for_existing(app: &mut App, index: usize) {
    let draft = app.config.regex[index].clone();
    app.regex_editor = Some(RegexEditorState {
        original_index: Some(index),
        draft,
        sample_input: String::new(),
        field: EditorField::Name,
        error: None,
    });
}

fn open_editor_for_new(app: &mut App) {
    app.regex_editor = Some(RegexEditorState {
        original_index: None,
        draft: RegexRule {
            name: String::new(),
            pattern: String::new(),
            replacement: String::new(),
            scope: vec![Scope::Display],
            target: vec![Target::Assistant],
            enabled: true,
            compile_error: None,
        },
        sample_input: String::new(),
        field: EditorField::Name,
        error: None,
    });
}

pub(in crate::tui) fn save_and_recompile(app: &mut App) {
    if let Err(err) = libllm::config::save(&app.config) {
        app.set_status(
            format!("Failed to save config: {err}"),
            crate::tui::types::StatusLevel::Error,
        );
        return;
    }
    app.compiled_regex = libllm::regex_rules::compile_rules(&app.config.regex);
    app.invalidate_chat_caches();
}
