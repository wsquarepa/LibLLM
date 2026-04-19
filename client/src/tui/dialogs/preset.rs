//! Preset picker and editor dialogs for instruct, reasoning, and template presets.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::ListItem;

use super::{clear_centered, render_hints_below_dialog};
use crate::tui::{App, DeleteContext, Focus};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PresetKind {
    Template,
    Instruct,
    Reasoning,
}

pub(in crate::tui) fn render_preset_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let names = &app.preset_picker_names;
    let count = names.len();
    let title = match app.preset_picker_kind {
        PresetKind::Template => " Select Template Preset ",
        PresetKind::Instruct => " Select Instruct Preset ",
        PresetKind::Reasoning => " Select Reasoning Preset ",
    };

    let height = super::paged_list_height(count, area.height, super::LIST_DIALOG_TALL_PADDING, false);
    let dialog = clear_centered(f, super::LIST_DIALOG_WIDTH, height, area);

    let items: Vec<ListItem<'_>> = names.iter().map(|name| ListItem::new(name.clone())).collect();

    super::render_paged_list(f, dialog, app.preset_picker_selected, items, title, &app.theme, None, None);

    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[
            Line::from("Up/Down: navigate  PgUp/PgDn: page  Home/End: jump"),
            Line::from("Enter: select  Right: edit  a: add  Del: delete  Esc: cancel"),
        ],
    );
}

pub(in crate::tui) fn handle_preset_dialog_key(
    key: KeyEvent,
    app: &mut App,
) -> Option<super::super::Action> {
    if app.preset_picker_names.is_empty() {
        match key.code {
            KeyCode::Char('a') => {
                create_and_edit_preset(app);
            }
            KeyCode::Esc => {
                app.focus = Focus::ConfigDialog;
            }
            _ => {}
        }
        return None;
    }

    let visible = super::page_size(app.last_terminal_height, super::LIST_DIALOG_TALL_PADDING);
    if super::handle_paged_list_key(
        &mut app.preset_picker_selected,
        app.preset_picker_names.len(),
        visible,
        key,
    ) == super::PagedListAction::Consumed
    {
        return None;
    }

    match key.code {
        KeyCode::Enter => {
            let chosen = app.preset_picker_names[app.preset_picker_selected].clone();
            apply_preset_selection(app, chosen);
            app.focus = Focus::ConfigDialog;
        }
        KeyCode::Right => {
            let name = app.preset_picker_names[app.preset_picker_selected].clone();
            if name == "OFF" || name == "Raw" {
                return None;
            }
            open_preset_editor(app, app.preset_picker_kind, &name);
        }
        KeyCode::Char('a') => {
            create_and_edit_preset(app);
        }
        KeyCode::Backspace | KeyCode::Delete => {
            let name = app.preset_picker_names[app.preset_picker_selected].clone();
            if name == "OFF" || name == "Raw" {
                return None;
            }
            app.delete_confirm_filename = name;
            app.delete_confirm_selected = 0;
            app.delete_context = DeleteContext::Preset {
                kind: app.preset_picker_kind,
            };
            app.focus = Focus::DeleteConfirmDialog;
        }
        KeyCode::Esc => {
            app.focus = Focus::ConfigDialog;
        }
        _ => {}
    }
    None
}

fn apply_preset_selection(app: &mut App, chosen: String) {
    let Some(ref mut dialog) = app.config_dialog else {
        return;
    };
    match app.preset_picker_kind {
        PresetKind::Template => dialog.set_value(0, 2, chosen),
        PresetKind::Instruct => dialog.set_value(0, 3, chosen),
        PresetKind::Reasoning => dialog.set_value(0, 4, chosen),
    }
}

pub(in crate::tui) fn open_preset_picker(app: &mut App, kind: PresetKind) {
    let names = match kind {
        PresetKind::Template => libllm::preset::list_template_preset_names(),
        PresetKind::Instruct => libllm::preset::list_instruct_preset_names(),
        PresetKind::Reasoning => libllm::preset::list_reasoning_preset_names(),
    };

    let current = app
        .config_dialog
        .as_ref()
        .map(|d| match kind {
            PresetKind::Template => d.sections()[0].values[2].as_str(),
            PresetKind::Instruct => d.sections()[0].values[3].as_str(),
            PresetKind::Reasoning => d.sections()[0].values[4].as_str(),
        })
        .unwrap_or("");

    let selected = names
        .iter()
        .position(|n| n.eq_ignore_ascii_case(current))
        .unwrap_or(0);

    app.preset_picker_kind = kind;
    app.preset_picker_names = names;
    app.preset_picker_selected = selected;
    app.focus = Focus::PresetPickerDialog;
}

fn open_preset_editor(app: &mut App, kind: PresetKind, name: &str) {
    match kind {
        PresetKind::Template => {
            let preset = libllm::preset::resolve_template_preset(name);
            let values = vec![
                preset.name.clone(),
                preset.story_string,
                preset.example_separator,
                preset.chat_start,
            ];
            app.preset_editor = Some(super::open_template_editor(values));
            app.preset_editor_original_name = preset.name;
        }
        PresetKind::Instruct => {
            let preset = libllm::preset::resolve_instruct_preset(name);
            let stop_str = match &preset.stop_sequence {
                libllm::preset::StopSequence::Single(s) => s.clone(),
                libllm::preset::StopSequence::Multiple(v) => v.join(", "),
            };
            let values = vec![
                preset.name.clone(),
                preset.input_sequence,
                preset.output_sequence,
                preset.system_sequence,
                preset.input_suffix,
                preset.output_suffix,
                preset.system_suffix,
                stop_str,
                preset.separator_sequence,
                preset.wrap.to_string(),
                preset.system_same_as_user.to_string(),
                preset.sequences_as_stop_strings.to_string(),
            ];
            app.preset_editor = Some(super::open_instruct_editor(values));
            app.preset_editor_original_name = preset.name;
        }
        PresetKind::Reasoning => {
            if let Some(preset) = libllm::preset::resolve_reasoning_preset(name) {
                let values = vec![
                    preset.name.clone(),
                    preset.prefix,
                    preset.suffix,
                    preset.separator,
                ];
                app.preset_editor = Some(super::open_reasoning_editor(values));
                app.preset_editor_original_name = preset.name;
            } else {
                return;
            }
        }
    }
    app.preset_editor_kind = kind;
    app.focus = Focus::PresetEditorDialog;
}

fn create_and_edit_preset(app: &mut App) {
    let kind = app.preset_picker_kind;
    let existing: std::collections::HashSet<String> =
        app.preset_picker_names.iter().cloned().collect();
    let base = match kind {
        PresetKind::Template => "template",
        PresetKind::Instruct => "instruct",
        PresetKind::Reasoning => "reasoning",
    };
    let new_name = super::generate_unique_name(base, &existing);

    match kind {
        PresetKind::Template => {
            let values = vec![
                new_name.clone(),
                String::new(),
                String::new(),
                String::new(),
            ];
            app.preset_editor = Some(super::open_template_editor(values));
        }
        PresetKind::Instruct => {
            let values = vec![
                new_name.clone(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                "false".to_owned(),
                "false".to_owned(),
                "false".to_owned(),
            ];
            app.preset_editor = Some(super::open_instruct_editor(values));
        }
        PresetKind::Reasoning => {
            let values = vec![
                new_name.clone(),
                String::new(),
                String::new(),
                String::new(),
            ];
            app.preset_editor = Some(super::open_reasoning_editor(values));
        }
    }

    app.preset_editor_kind = kind;
    app.preset_editor_original_name = String::new();
    app.preset_picker_names.push(new_name);
    app.preset_picker_selected = app.preset_picker_names.len() - 1;
    app.focus = Focus::PresetEditorDialog;
}

fn dir_for_kind(kind: PresetKind) -> std::path::PathBuf {
    match kind {
        PresetKind::Template => libllm::preset::template_presets_dir(),
        PresetKind::Instruct => libllm::preset::instruct_presets_dir(),
        PresetKind::Reasoning => libllm::preset::reasoning_presets_dir(),
    }
}

fn sanitize_preset_name(raw: &str) -> Option<String> {
    let safe: String = raw.replace(['/', '\\'], "_");
    let safe = safe.trim_matches('.');
    if safe.is_empty() {
        None
    } else {
        Some(safe.to_owned())
    }
}

pub(in crate::tui) fn save_preset_from_editor(
    kind: PresetKind,
    values: &[String],
    original_name: &str,
) -> anyhow::Result<()> {
    let name = sanitize_preset_name(values[0].trim()).unwrap_or_default();
    if name.is_empty() {
        anyhow::bail!("preset name cannot be empty");
    }
    if name.eq_ignore_ascii_case("OFF") {
        anyhow::bail!("'OFF' is a reserved name");
    }

    let dir = dir_for_kind(kind);
    let json = match kind {
        PresetKind::Template => serde_json::to_string_pretty(&serde_json::json!({
            "name": name,
            "story_string": values[1],
            "example_separator": values[2],
            "chat_start": values[3],
        }))?,
        PresetKind::Instruct => {
            let stop_seqs: Vec<&str> = values[7]
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            serde_json::to_string_pretty(&serde_json::json!({
                "name": name,
                "input_sequence": values[1],
                "output_sequence": values[2],
                "system_sequence": values[3],
                "input_suffix": values[4],
                "output_suffix": values[5],
                "system_suffix": values[6],
                "stop_sequence": stop_seqs,
                "separator_sequence": values[8],
                "wrap": values[9].parse::<bool>().unwrap_or(false),
                "system_same_as_user": values[10].parse::<bool>().unwrap_or(false),
                "sequences_as_stop_strings": values[11].parse::<bool>().unwrap_or(false),
            }))?
        }
        PresetKind::Reasoning => serde_json::to_string_pretty(&serde_json::json!({
            "name": name,
            "prefix": values[1],
            "suffix": values[2],
            "separator": values[3],
        }))?,
    };

    let path = dir.join(format!("{name}.json"));
    anyhow::ensure!(
        path.starts_with(&dir),
        "preset path escapes target directory"
    );
    std::fs::write(&path, json)?;

    if !original_name.is_empty() && original_name != name
        && let Some(safe_original) = sanitize_preset_name(original_name) {
            let old_path = dir.join(format!("{safe_original}.json"));
            if old_path.starts_with(&dir) && old_path != path {
                let _ = std::fs::remove_file(&old_path);
            }
        }

    Ok(())
}

pub(in crate::tui) fn delete_preset(kind: PresetKind, name: &str) {
    let dir = dir_for_kind(kind);
    if let Some(safe_name) = sanitize_preset_name(name) {
        let path = dir.join(format!("{safe_name}.json"));
        if path.starts_with(&dir) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

pub(in crate::tui) fn refresh_preset_list(app: &mut App) {
    let names = match app.preset_picker_kind {
        PresetKind::Template => libllm::preset::list_template_preset_names(),
        PresetKind::Instruct => libllm::preset::list_instruct_preset_names(),
        PresetKind::Reasoning => libllm::preset::list_reasoning_preset_names(),
    };
    app.preset_picker_selected = app
        .preset_picker_selected
        .min(names.len().saturating_sub(1));
    app.preset_picker_names = names;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_preset_does_not_delete_file_when_sanitized_names_collide() {
        let dir = tempfile::tempdir().expect("temp dir");
        libllm::config::set_data_dir(dir.path().to_path_buf()).ok();

        let preset_dir = libllm::preset::instruct_presets_dir();
        std::fs::create_dir_all(&preset_dir).expect("create preset dir");

        let values = vec![
            "foo".to_owned(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            "false".to_owned(),
            "false".to_owned(),
            "false".to_owned(),
        ];

        save_preset_from_editor(PresetKind::Instruct, &values, ".foo")
            .expect("save should succeed");

        let saved_path = preset_dir.join("foo.json");
        assert!(
            saved_path.exists(),
            "preset file must not be deleted when sanitized old and new names are the same"
        );
    }
}
