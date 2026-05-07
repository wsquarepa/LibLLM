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
    let editor_height: u16 = if app.regex_editor.is_some() { 14 } else { 0 };
    let list_height = super::paged_list_height(
        count,
        area.height.saturating_sub(editor_height),
        super::FIELD_DIALOG_PADDING_ROWS,
    );
    let total_height = list_height + editor_height;
    let dialog = clear_centered(f, super::LIST_DIALOG_WIDTH, total_height, area);

    let split = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(list_height),
            ratatui::layout::Constraint::Length(editor_height),
        ])
        .split(dialog);

    let items: Vec<ListItem<'_>> = labels.iter().cloned().map(ListItem::new).collect();
    super::render_paged_list(
        f,
        split[0],
        &app.theme,
        super::PagedListContent {
            selected: app.regex_list_selected,
            items,
            title_base: " Regex rules ",
            search: None,
            unfiltered_total: None,
        },
    );

    if let Some(ed) = app.regex_editor.as_ref() {
        render_editor_pane(f, split[1], ed);
    }

    let hints = if app.regex_editor.is_some() {
        vec![Line::from(
            "Tab: next field  Space: toggle  Enter: edit text  Ctrl+S: save  Esc: cancel",
        )]
    } else {
        vec![
            Line::from("Up/Down: navigate  Space: toggle  Enter: edit  n: new  d: delete"),
            Line::from("Shift+Up/Down: reorder  Esc: close"),
        ]
    };
    render_hints_below_dialog(f, dialog, area, &hints);
}

fn render_editor_pane(f: &mut ratatui::Frame, area: Rect, ed: &RegexEditorState) {
    let cursor = |field: EditorField| -> &'static str {
        if ed.field == field { ">" } else { " " }
    };
    let scope_marks = |s: Scope| -> &'static str {
        if ed.draft.scope.contains(&s) { "[x]" } else { "[ ]" }
    };
    let target_marks = |t: Target| -> &'static str {
        if ed.draft.target.contains(&t) { "[x]" } else { "[ ]" }
    };
    let preview = libllm::regex_rules::compile_rules(std::slice::from_ref(&ed.draft));
    let preview_out = if preview.is_empty() {
        "(invalid pattern)".to_owned()
    } else {
        let scope = ed.draft.scope.first().copied().unwrap_or(Scope::Display);
        let role = match ed.draft.target.first().copied().unwrap_or(Target::User) {
            Target::User => libllm::session::Role::User,
            Target::Assistant => libllm::session::Role::Assistant,
            Target::System => libllm::session::Role::System,
            Target::Summary => libllm::session::Role::Summary,
        };
        libllm::regex_rules::apply(&preview, scope, role, &ed.sample_input).into_owned()
    };

    let lines: Vec<Line<'_>> = vec![
        Line::from(format!(
            "{} name:        {}",
            cursor(EditorField::Name),
            ed.draft.name
        )),
        Line::from(format!(
            "{} pattern:     {}",
            cursor(EditorField::Pattern),
            ed.draft.pattern
        )),
        Line::from(format!(
            "{} replacement: {}",
            cursor(EditorField::Replacement),
            ed.draft.replacement
        )),
        Line::from(format!(
            "{} scope:       {}display {}send {}recv {}export",
            cursor(EditorField::ScopeToggles),
            scope_marks(Scope::Display),
            scope_marks(Scope::PromptSend),
            scope_marks(Scope::PromptRecv),
            scope_marks(Scope::Export),
        )),
        Line::from(format!(
            "{} target:      {}user {}asst {}sys {}sum",
            cursor(EditorField::TargetToggles),
            target_marks(Target::User),
            target_marks(Target::Assistant),
            target_marks(Target::System),
            target_marks(Target::Summary),
        )),
        Line::from(format!(
            "{} enabled:     {}",
            cursor(EditorField::Enabled),
            if ed.draft.enabled { "[x]" } else { "[ ]" }
        )),
        Line::from(format!(
            "{} sample:      {}",
            cursor(EditorField::SampleInput),
            ed.sample_input
        )),
        Line::from(format!("  preview:     {preview_out}")),
        Line::from(ed.error.clone().unwrap_or_default()),
    ];

    let block = ratatui::widgets::Paragraph::new(lines).block(
        ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .title(" Edit "),
    );
    f.render_widget(block, area);
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
            app.delete_context = crate::tui::types::DeleteContext::Regex;
            app.delete_confirm_selected = 0;
            app.delete_confirm_filename = app.config.regex[i].name.clone();
            app.focus = crate::tui::Focus::DeleteConfirmDialog;
        }
        _ => {}
    }
    None
}

fn handle_editor_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    let ed = app.regex_editor.as_mut()?;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match (key.code, ctrl) {
        (KeyCode::Esc, _) => {
            app.regex_editor = None;
        }
        (KeyCode::Char('s'), true) => {
            commit_editor(app);
        }
        (KeyCode::Tab, _) => {
            ed.field = ed.field.next();
        }
        (KeyCode::Char(' '), false) => match ed.field {
            EditorField::ScopeToggles => cycle_scope(ed),
            EditorField::TargetToggles => cycle_target(ed),
            EditorField::Enabled => ed.draft.enabled = !ed.draft.enabled,
            _ => append_char(ed, ' '),
        },
        (KeyCode::Char(c), false) => append_char(ed, c),
        (KeyCode::Backspace, _) => backspace(ed),
        _ => {}
    }
    None
}

fn append_char(ed: &mut RegexEditorState, c: char) {
    match ed.field {
        EditorField::Name => ed.draft.name.push(c),
        EditorField::Pattern => ed.draft.pattern.push(c),
        EditorField::Replacement => ed.draft.replacement.push(c),
        EditorField::SampleInput => ed.sample_input.push(c),
        EditorField::ScopeToggles | EditorField::TargetToggles | EditorField::Enabled => {}
    }
}

fn backspace(ed: &mut RegexEditorState) {
    let target: Option<&mut String> = match ed.field {
        EditorField::Name => Some(&mut ed.draft.name),
        EditorField::Pattern => Some(&mut ed.draft.pattern),
        EditorField::Replacement => Some(&mut ed.draft.replacement),
        EditorField::SampleInput => Some(&mut ed.sample_input),
        _ => None,
    };
    if let Some(s) = target {
        s.pop();
    }
}

fn cycle_scope(ed: &mut RegexEditorState) {
    let order = [
        Scope::Display,
        Scope::PromptSend,
        Scope::PromptRecv,
        Scope::Export,
    ];
    let idx = order
        .iter()
        .position(|s| !ed.draft.scope.contains(s))
        .unwrap_or(0);
    let next = order[idx];
    if let Some(pos) = ed.draft.scope.iter().position(|s| *s == next) {
        ed.draft.scope.remove(pos);
    } else {
        ed.draft.scope.push(next);
    }
}

fn cycle_target(ed: &mut RegexEditorState) {
    let order = [
        Target::User,
        Target::Assistant,
        Target::System,
        Target::Summary,
    ];
    let idx = order
        .iter()
        .position(|t| !ed.draft.target.contains(t))
        .unwrap_or(0);
    let next = order[idx];
    if let Some(pos) = ed.draft.target.iter().position(|t| *t == next) {
        ed.draft.target.remove(pos);
    } else {
        ed.draft.target.push(next);
    }
}

fn validate_pattern(rule: &mut RegexRule) -> Option<String> {
    rule.compile_error = None;
    if let Err(err) = regex::Regex::new(&rule.pattern) {
        let msg = format!("invalid pattern: {err}");
        rule.compile_error = Some(msg.clone());
        rule.enabled = false;
        return Some(msg);
    }
    None
}

fn commit_editor(app: &mut App) {
    let Some(mut ed) = app.regex_editor.take() else {
        return;
    };
    if let Some(msg) = validate_pattern(&mut ed.draft) {
        app.set_status(
            format!("Saved with errors: {msg}"),
            crate::tui::types::StatusLevel::Warning,
        );
    }
    match ed.original_index {
        Some(i) => app.config.regex[i] = ed.draft,
        None => app.config.regex.push(ed.draft),
    }
    save_and_recompile(app);
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

pub(in crate::tui) fn perform_delete_selected(app: &mut App) {
    let i = app.regex_list_selected;
    if i >= app.config.regex.len() {
        return;
    }
    app.config.regex.remove(i);
    if app.regex_list_selected >= app.config.regex.len() && app.regex_list_selected > 0 {
        app.regex_list_selected -= 1;
    }
    save_and_recompile(app);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_pattern_marks_invalid_rule_disabled() {
        let mut rule = RegexRule {
            name: "bad".to_owned(),
            pattern: "(unclosed".to_owned(),
            replacement: String::new(),
            scope: vec![Scope::Display],
            target: vec![Target::Assistant],
            enabled: true,
            compile_error: None,
        };
        let result = validate_pattern(&mut rule);
        assert!(result.is_some(), "invalid pattern should produce an error message");
        assert!(!rule.enabled, "invalid pattern must disable the rule");
        assert!(rule.compile_error.is_some(), "compile_error must be set");
    }

    #[test]
    fn validate_pattern_clears_compile_error_on_valid_pattern() {
        let mut rule = RegexRule {
            name: "good".to_owned(),
            pattern: "x".to_owned(),
            replacement: "y".to_owned(),
            scope: vec![Scope::Display],
            target: vec![Target::Assistant],
            enabled: true,
            compile_error: Some("stale error".to_owned()),
        };
        let result = validate_pattern(&mut rule);
        assert!(result.is_none(), "valid pattern should return None");
        assert!(rule.compile_error.is_none(), "stale compile_error must be cleared");
        assert!(rule.enabled, "valid pattern must not change enabled flag");
    }

    #[test]
    fn cycle_scope_toggles_each_scope_once() {
        let mut state = RegexEditorState {
            original_index: None,
            draft: RegexRule {
                name: "x".to_owned(),
                pattern: "x".to_owned(),
                replacement: "y".to_owned(),
                scope: Vec::new(),
                target: vec![Target::Assistant],
                enabled: true,
                compile_error: None,
            },
            sample_input: String::new(),
            field: EditorField::ScopeToggles,
            error: None,
        };
        cycle_scope(&mut state);
        cycle_scope(&mut state);
        cycle_scope(&mut state);
        cycle_scope(&mut state);
        assert_eq!(state.draft.scope.len(), 4);
    }
}
