//! Pure snapshot extraction from `App` state.

use serde::Serialize;

use libllm_core::session::Role;
use libllm_protocol::tokenizer::CountState;

use crate::types::{App, Focus, StatusLevel};

/// A total, serializable snapshot of observable TUI state at a single instant.
///
/// Every field is populated without mutating `app` and without panicking.
/// Fields that are structurally absent return `None` or an empty default.
/// `branch_label` is always `None` because the status bar does not render a
/// branch indicator — the information is not surfaced in `App` state at a level
/// that can be read purely.
#[derive(Debug, Clone, Serialize)]
pub struct Observation {
    pub focus: Focus,
    pub active_dialog: Option<String>,
    pub dialog_dirty: bool,
    pub model: Option<String>,
    pub template: Option<String>,
    pub token_count: Option<usize>,
    pub branch_label: Option<String>,
    pub status_message: Option<String>,
    pub status_level: Option<StatusLevel>,
    pub is_streaming: bool,
    pub is_summarizing: bool,
    pub message_count: usize,
    pub head_role: Option<Role>,
    pub head_text: Option<String>,
    pub sidebar_session_count: usize,
    pub input_text: String,
    pub message_queue_len: usize,
}

/// Maps a `Focus` variant to a stable dialog identifier, or `None` for the
/// non-dialog focus variants (`Input`, `Chat`, `Sidebar`).
///
/// The match is exhaustive so a newly added `Focus` variant is a compile error
/// here rather than a silently missing observation.
pub(crate) fn active_dialog_name_for(focus: Focus) -> Option<String> {
    let name = match focus {
        Focus::PasskeyDialog => "passkey",
        Focus::SetPasskeyDialog => "set_passkey",
        Focus::ConfigDialog => "config",
        Focus::ThemeDialog => "theme",
        Focus::BaseThemePickerDialog => "base_theme_picker",
        Focus::PresetPickerDialog => "preset_picker",
        Focus::AuthDialog => "auth_dialog",
        Focus::AuthTypePicker => "auth_type_picker",
        Focus::PresetEditorDialog => "preset_editor",
        Focus::PersonaDialog => "persona",
        Focus::PersonaEditorDialog => "persona_editor",
        Focus::AuthorNoteEditorDialog => "author_note_editor",
        Focus::CharacterDialog => "character",
        Focus::CharacterEditorDialog => "character_editor",
        Focus::WorldbookDialog => "worldbook",
        Focus::WorldbookEditorDialog => "worldbook_editor",
        Focus::WorldbookEntryEditorDialog => "worldbook_entry_editor",
        Focus::WorldbookEntryDeleteDialog => "worldbook_entry_delete",
        Focus::SystemPromptDialog => "system_prompt",
        Focus::SystemPromptEditorDialog => "system_prompt_editor",
        Focus::EditDialog => "edit",
        Focus::UnsavedWarningDialog => "unsaved_warning",
        Focus::BranchDialog => "branch",
        Focus::SearchDialog => "search",
        Focus::RegexDialog => "regex",
        Focus::RegexEditorDialog => "regex_editor",
        Focus::DeleteConfirmDialog => "delete_confirm",
        Focus::ApiErrorDialog => "api_error",
        Focus::FilePickerDialog => "file_picker",
        Focus::FileReferenceConfirmDialog => "file_reference_confirm",
        Focus::InjectionWarningDialog => "injection_warning",
        Focus::LoadingDialog => "loading",
        Focus::TemplatePromptDialog => "template_prompt",
        Focus::DangerConfirmDialog => "danger_confirm",
        Focus::DangerTypedConfirmDialog => "danger_typed_confirm",
        Focus::ChatSettingsDialog => "chat_settings",
        Focus::ScenarioEditorDialog => "scenario_editor",
        Focus::Input | Focus::Chat | Focus::Sidebar => return None,
    };
    Some(name.to_owned())
}

fn dialog_dirty_for(app: &App) -> bool {
    match app.focus {
        Focus::ConfigDialog => app.config_dialog.as_ref().is_some_and(|d| d.has_changes()),
        Focus::ThemeDialog => app.theme_dialog.as_ref().is_some_and(|d| d.has_changes()),
        Focus::PresetEditorDialog => app.preset_editor.as_ref().is_some_and(|d| d.has_changes()),
        Focus::PersonaEditorDialog => app.persona_editor.as_ref().is_some_and(|d| d.has_changes()),
        Focus::AuthorNoteEditorDialog => app
            .author_note_editor
            .as_ref()
            .is_some_and(|d| d.has_changes()),
        Focus::CharacterEditorDialog => app
            .character_editor
            .as_ref()
            .is_some_and(|d| d.has_changes()),
        Focus::WorldbookEntryEditorDialog => app
            .worldbook_entry_editor
            .as_ref()
            .is_some_and(|d| d.has_changes()),
        Focus::SystemPromptEditorDialog => app
            .system_prompt_editor
            .as_ref()
            .is_some_and(|d| d.has_changes()),
        Focus::RegexEditorDialog => app.regex_editor.as_ref().is_some_and(|d| d.is_dirty()),
        _ => false,
    }
}

/// Returns a total snapshot of observable `App` state.
///
/// Performs no mutations, spawns nothing, and never panics.
pub(crate) fn observe(app: &App) -> Observation {
    let focus = app.focus;
    let active_dialog = active_dialog_name_for(focus);
    let dialog_dirty = dialog_dirty_for(app);
    let model = app.model_name.clone();
    let template = Some(app.instruct_preset.name.clone());
    let token_count = app.cached_token_count.map(|state| match state {
        CountState::Authoritative(n) => n,
        CountState::Stale(n) => n,
        CountState::Estimated(n) => n,
    });
    let status_message = app.status_message.as_ref().map(|m| m.text.clone());
    let status_level = app.status_message.as_ref().map(|m| m.level);
    let message_count = app.session.tree.current_branch_ids().len();
    let (head_role, head_text) = app
        .session
        .tree
        .head()
        .and_then(|id| app.session.tree.node(id))
        .map(|node| (Some(node.message.role), Some(node.message.content.clone())))
        .unwrap_or((None, None));

    Observation {
        focus,
        active_dialog,
        dialog_dirty,
        model,
        template,
        token_count,
        branch_label: None,
        status_message,
        status_level,
        is_streaming: app.streaming.active,
        is_summarizing: app.is_summarizing,
        message_count,
        head_role,
        head_text,
        sidebar_session_count: app.sidebar_sessions.len(),
        input_text: app.textarea.lines().join("\n"),
        message_queue_len: app.streaming.message_queue.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_name_is_none_for_input_focus() {
        assert_eq!(active_dialog_name_for(Focus::Input), None);
    }

    #[test]
    fn dialog_name_is_none_for_chat_focus() {
        assert_eq!(active_dialog_name_for(Focus::Chat), None);
    }

    #[test]
    fn dialog_name_is_none_for_sidebar_focus() {
        assert_eq!(active_dialog_name_for(Focus::Sidebar), None);
    }

    #[test]
    fn dialog_name_is_set_for_persona_dialog() {
        assert_eq!(
            active_dialog_name_for(Focus::PersonaDialog),
            Some("persona".to_owned())
        );
    }

    #[test]
    fn dialog_name_is_set_for_config_dialog() {
        assert_eq!(
            active_dialog_name_for(Focus::ConfigDialog),
            Some("config".to_owned())
        );
    }

    #[test]
    fn dialog_name_is_set_for_unsaved_warning() {
        assert_eq!(
            active_dialog_name_for(Focus::UnsavedWarningDialog),
            Some("unsaved_warning".to_owned())
        );
    }
}
