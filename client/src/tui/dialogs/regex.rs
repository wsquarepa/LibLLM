//! Manage regex find/replace rules: list view and a centered editor modal.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::ListItem;

use libllm::regex_rules::{RegexRule, Scope, Target};

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::tui::dialog_handler::return_to_input;
use crate::tui::{Action, App, Focus};

const REGEX_EDITOR_DIALOG_WIDTH: u16 = 72;
const REGEX_EDITOR_DIALOG_HEIGHT: u16 = 18;

const SCOPE_ORDER: [Scope; 4] = [
    Scope::Display,
    Scope::PromptSend,
    Scope::PromptRecv,
    Scope::Export,
];
const SCOPE_LABELS: [&str; 4] = ["display", "send", "recv", "export"];

const TARGET_ORDER: [Target; 4] = [
    Target::User,
    Target::Assistant,
    Target::System,
    Target::Summary,
];
const TARGET_LABELS: [&str; 4] = ["user", "asst", "sys", "sum"];

pub struct RegexEditorState {
    pub original_index: Option<usize>,
    pub draft: RegexRule,
    pub sample_input: String,
    pub field: EditorField,
    pub scope_cursor: usize,
    pub target_cursor: usize,
    pub editing: bool,
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

    fn prev(self) -> Self {
        match self {
            Self::Name => Self::SampleInput,
            Self::Pattern => Self::Name,
            Self::Replacement => Self::Pattern,
            Self::ScopeToggles => Self::Replacement,
            Self::TargetToggles => Self::ScopeToggles,
            Self::Enabled => Self::TargetToggles,
            Self::SampleInput => Self::Enabled,
        }
    }

    fn is_text(self) -> bool {
        matches!(
            self,
            Self::Name | Self::Pattern | Self::Replacement | Self::SampleInput
        )
    }
}

pub(in crate::tui) fn open(app: &mut App) {
    app.regex_list_selected = app
        .regex_list_selected
        .min(app.config.regex.len().saturating_sub(1));
    app.regex_editor = None;
    app.focus = Focus::RegexDialog;
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

pub(in crate::tui) fn render_regex_editor_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let Some(ed) = app.regex_editor.as_ref() else {
        return;
    };
    let dialog = clear_centered(
        f,
        REGEX_EDITOR_DIALOG_WIDTH,
        REGEX_EDITOR_DIALOG_HEIGHT,
        area,
    );
    let title = if ed.original_index.is_some() {
        " Edit regex rule "
    } else {
        " New regex rule "
    };
    f.render_widget(dialog_block(title, app.theme.border_focused), dialog);

    let fields_area = Rect {
        x: dialog.x + 2,
        y: dialog.y + 1,
        width: dialog.width.saturating_sub(4),
        height: 8,
    };
    f.render_widget(
        ratatui::widgets::Paragraph::new(build_editor_lines(ed, app)),
        fields_area,
    );

    let preview_area = Rect {
        x: dialog.x + 2,
        y: dialog.y + dialog.height.saturating_sub(6),
        width: dialog.width.saturating_sub(4),
        height: 4,
    };
    render_preview_box(f, preview_area, ed, app);

    if let Some(err) = ed.error.as_ref() {
        let err_area = Rect {
            x: dialog.x + 2,
            y: dialog.y + dialog.height.saturating_sub(2),
            width: dialog.width.saturating_sub(4),
            height: 1,
        };
        f.render_widget(
            ratatui::widgets::Paragraph::new(Line::from(Span::styled(
                err.clone(),
                Style::default().fg(app.theme.status_error_fg),
            ))),
            err_area,
        );
    }

    let hints = if ed.editing {
        vec![Line::from("Type to edit  Enter/Esc: stop editing")]
    } else if matches!(ed.field, EditorField::ScopeToggles | EditorField::TargetToggles) {
        vec![Line::from(
            "Left/Right: option  Space/Enter: toggle  Up/Down/Tab: field  Esc: save & close",
        )]
    } else {
        vec![Line::from(
            "Up/Down: field  Enter: edit/toggle  Space: toggle  Esc: save & close",
        )]
    };
    render_hints_below_dialog(f, dialog, area, &hints);
}

fn render_preview_box(f: &mut ratatui::Frame, area: Rect, ed: &RegexEditorState, app: &App) {
    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .title(" preview ")
        .border_style(Style::default().fg(app.theme.border_unfocused));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let preview_out = compute_preview(&ed.draft, &ed.sample_input);
    f.render_widget(
        ratatui::widgets::Paragraph::new(preview_out)
            .style(Style::default().fg(app.theme.dimmed))
            .wrap(ratatui::widgets::Wrap { trim: false }),
        inner,
    );
}

fn build_editor_lines(ed: &RegexEditorState, _app: &App) -> Vec<Line<'static>> {
    vec![
        text_row(ed, EditorField::Name, "name:", &ed.draft.name),
        text_row(ed, EditorField::Pattern, "pattern:", &ed.draft.pattern),
        text_row(
            ed,
            EditorField::Replacement,
            "replacement:",
            &ed.draft.replacement,
        ),
        toggle_row(
            ed,
            EditorField::ScopeToggles,
            "scope:",
            &SCOPE_LABELS,
            ed.scope_cursor,
            |i| ed.draft.scope.contains(&SCOPE_ORDER[i]),
        ),
        toggle_row(
            ed,
            EditorField::TargetToggles,
            "target:",
            &TARGET_LABELS,
            ed.target_cursor,
            |i| ed.draft.target.contains(&TARGET_ORDER[i]),
        ),
        bool_row(ed, EditorField::Enabled, "enabled:", ed.draft.enabled),
        text_row(
            ed,
            EditorField::SampleInput,
            "sample:",
            &ed.sample_input,
        ),
    ]
}

fn cursor_marker(ed: &RegexEditorState, field: EditorField) -> &'static str {
    if ed.field == field { "> " } else { "  " }
}

fn label_style(ed: &RegexEditorState, field: EditorField) -> Style {
    if ed.field == field {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn value_style(ed: &RegexEditorState, field: EditorField) -> Style {
    if ed.field == field {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    }
}

fn text_row(
    ed: &RegexEditorState,
    field: EditorField,
    label: &str,
    value: &str,
) -> Line<'static> {
    let display = if ed.field == field && ed.editing && field.is_text() {
        format!("{value}_")
    } else {
        value.to_owned()
    };
    Line::from(vec![
        Span::styled(
            format!("{}{:<13}", cursor_marker(ed, field), label),
            label_style(ed, field),
        ),
        Span::styled(display, value_style(ed, field)),
    ])
}

fn bool_row(
    ed: &RegexEditorState,
    field: EditorField,
    label: &str,
    val: bool,
) -> Line<'static> {
    let mark = if val { "[x]" } else { "[ ]" };
    Line::from(vec![
        Span::styled(
            format!("{}{:<13}", cursor_marker(ed, field), label),
            label_style(ed, field),
        ),
        Span::styled(mark.to_owned(), value_style(ed, field)),
    ])
}

fn toggle_row(
    ed: &RegexEditorState,
    field: EditorField,
    label: &str,
    options: &[&str],
    sub_cursor: usize,
    is_set: impl Fn(usize) -> bool,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::styled(
        format!("{}{:<13}", cursor_marker(ed, field), label),
        label_style(ed, field),
    )];

    let on_field = ed.field == field;
    for (i, name) in options.iter().enumerate() {
        let mark = if is_set(i) { "[x] " } else { "[ ] " };
        let token = format!("{mark}{name}");
        let style = if on_field && i == sub_cursor {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if on_field {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        spans.push(Span::styled(token, style));
        if i + 1 < options.len() {
            spans.push(Span::raw(" "));
        }
    }
    Line::from(spans)
}

fn compute_preview(draft: &RegexRule, sample: &str) -> String {
    let preview = libllm::regex_rules::compile_rules(std::slice::from_ref(draft));
    if preview.is_empty() {
        return "(invalid pattern)".to_owned();
    }
    let scope = draft.scope.first().copied().unwrap_or(Scope::Display);
    let role = match draft.target.first().copied().unwrap_or(Target::User) {
        Target::User => libllm::session::Role::User,
        Target::Assistant => libllm::session::Role::Assistant,
        Target::System => libllm::session::Role::System,
        Target::Summary => libllm::session::Role::Summary,
    };
    libllm::regex_rules::apply(&preview, scope, role, sample).into_owned()
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
            app.focus = Focus::DeleteConfirmDialog;
        }
        _ => {}
    }
    None
}

pub(in crate::tui) fn handle_regex_editor_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    let Some(ed) = app.regex_editor.as_mut() else {
        app.focus = Focus::RegexDialog;
        return None;
    };

    if ed.editing {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                ed.editing = false;
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                append_char(ed, c);
            }
            KeyCode::Backspace => backspace(ed),
            _ => {}
        }
        return None;
    }

    match key.code {
        KeyCode::Esc => {
            commit_editor(app);
            app.focus = Focus::RegexDialog;
        }
        KeyCode::Up => {
            if let Some(ed) = app.regex_editor.as_mut() {
                ed.field = ed.field.prev();
            }
        }
        KeyCode::Down | KeyCode::Tab => {
            if let Some(ed) = app.regex_editor.as_mut() {
                ed.field = ed.field.next();
            }
        }
        KeyCode::Left => {
            if let Some(ed) = app.regex_editor.as_mut() {
                match ed.field {
                    EditorField::ScopeToggles => {
                        ed.scope_cursor = ed.scope_cursor.saturating_sub(1);
                    }
                    EditorField::TargetToggles => {
                        ed.target_cursor = ed.target_cursor.saturating_sub(1);
                    }
                    _ => {}
                }
            }
        }
        KeyCode::Right => {
            if let Some(ed) = app.regex_editor.as_mut() {
                match ed.field {
                    EditorField::ScopeToggles
                        if ed.scope_cursor + 1 < SCOPE_ORDER.len() =>
                    {
                        ed.scope_cursor += 1;
                    }
                    EditorField::TargetToggles
                        if ed.target_cursor + 1 < TARGET_ORDER.len() =>
                    {
                        ed.target_cursor += 1;
                    }
                    _ => {}
                }
            }
        }
        KeyCode::Char(' ') | KeyCode::Enter
            if matches!(
                app.regex_editor.as_ref().map(|e| e.field),
                Some(EditorField::ScopeToggles)
                    | Some(EditorField::TargetToggles)
                    | Some(EditorField::Enabled)
            ) =>
        {
            if let Some(ed) = app.regex_editor.as_mut() {
                match ed.field {
                    EditorField::ScopeToggles => toggle_scope_at_cursor(ed),
                    EditorField::TargetToggles => toggle_target_at_cursor(ed),
                    EditorField::Enabled => ed.draft.enabled = !ed.draft.enabled,
                    _ => {}
                }
            }
        }
        KeyCode::Enter => {
            if let Some(ed) = app.regex_editor.as_mut()
                && ed.field.is_text()
            {
                ed.editing = true;
            }
        }
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
        _ => {}
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

fn toggle_scope_at_cursor(ed: &mut RegexEditorState) {
    let scope = SCOPE_ORDER[ed.scope_cursor];
    if let Some(pos) = ed.draft.scope.iter().position(|s| *s == scope) {
        ed.draft.scope.remove(pos);
    } else {
        ed.draft.scope.push(scope);
    }
}

fn toggle_target_at_cursor(ed: &mut RegexEditorState) {
    let target = TARGET_ORDER[ed.target_cursor];
    if let Some(pos) = ed.draft.target.iter().position(|t| *t == target) {
        ed.draft.target.remove(pos);
    } else {
        ed.draft.target.push(target);
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
        scope_cursor: 0,
        target_cursor: 0,
        editing: false,
        error: None,
    });
    app.focus = Focus::RegexEditorDialog;
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
        scope_cursor: 0,
        target_cursor: 0,
        editing: false,
        error: None,
    });
    app.focus = Focus::RegexEditorDialog;
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
    fn toggle_scope_at_cursor_only_toggles_pointed_item() {
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
            scope_cursor: 2,
            target_cursor: 0,
            editing: false,
            error: None,
        };
        toggle_scope_at_cursor(&mut state);
        assert_eq!(state.draft.scope, vec![Scope::PromptRecv]);
        toggle_scope_at_cursor(&mut state);
        assert!(state.draft.scope.is_empty());
    }

    #[test]
    fn toggle_target_at_cursor_only_toggles_pointed_item() {
        let mut state = RegexEditorState {
            original_index: None,
            draft: RegexRule {
                name: "x".to_owned(),
                pattern: "x".to_owned(),
                replacement: "y".to_owned(),
                scope: vec![Scope::Display],
                target: Vec::new(),
                enabled: true,
                compile_error: None,
            },
            sample_input: String::new(),
            field: EditorField::TargetToggles,
            scope_cursor: 0,
            target_cursor: 1,
            editing: false,
            error: None,
        };
        toggle_target_at_cursor(&mut state);
        assert_eq!(state.draft.target, vec![Target::Assistant]);
    }
}
