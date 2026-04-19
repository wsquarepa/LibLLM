//! Mouse hit-testing for dialog list items and field editor regions.

use crossterm::event::MouseEvent;
use ratatui::layout::{Position, Rect};

use super::{DIALOG_HEIGHT_RATIO, DIALOG_WIDTH_RATIO, FIELD_DIALOG_DEFAULT_WIDTH, LIST_DIALOG_TALL_PADDING, LIST_DIALOG_WIDTH};
use super::SearchState;

pub(in crate::tui) enum ListDialogHit {
    Item(usize),
    Outside,
    Inside,
    SearchTitle,
}

pub(in crate::tui) fn hit_test_list_dialog(
    dialog: Rect,
    visible_indices: &[usize],
    current_selected: usize,
    screen_col: u16,
    screen_row: u16,
    search: Option<&SearchState>,
) -> ListDialogHit {
    let pos = Position::new(screen_col, screen_row);
    if !dialog.contains(pos) {
        return ListDialogHit::Outside;
    }
    if let Some(state) = search
        && screen_row + 1 == dialog.y + dialog.height
        && hit_search_region(state, dialog, screen_col)
    {
        return ListDialogHit::SearchTitle;
    }
    let items_area = Rect {
        x: dialog.x + 1,
        y: dialog.y + 1,
        width: dialog.width.saturating_sub(2),
        height: dialog.height.saturating_sub(2),
    };
    match super::map_list_click(items_area, visible_indices, current_selected, screen_row) {
        Some(orig) => ListDialogHit::Item(orig),
        None => ListDialogHit::Inside,
    }
}

fn hit_search_region(state: &SearchState, container: Rect, click_col: u16) -> bool {
    let max = container.width.saturating_sub(2);
    let width = super::search_title_width(state, max);
    let left_edge = container.x;
    let right_edge = left_edge + width;
    click_col >= left_edge && click_col < right_edge
}

fn activate_search_for_dialog(state: &mut SearchState, current_selected: usize) {
    if !state.active {
        state.enter(current_selected);
    }
}

pub(in crate::tui) fn handle_dialog_mouse_click(mouse: MouseEvent, app: &mut crate::tui::App) {
    let terminal_area = match crossterm::terminal::size() {
        Ok((w, h)) => Rect::new(0, 0, w, h),
        Err(_) => return,
    };

    match app.focus {
        crate::tui::Focus::CharacterDialog => {
            let indices = super::filter_indices(&app.character_names, &app.dialog_search);
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                LIST_DIALOG_TALL_PADDING,
                LIST_DIALOG_WIDTH,
            );
            match hit_test_list_dialog(
                dialog,
                &indices,
                app.character_selected,
                mouse.column,
                mouse.row,
                Some(&app.dialog_search),
            ) {
                ListDialogHit::Item(i) => app.character_selected = i,
                ListDialogHit::Outside => {
                    app.dialog_search.commit();
                    app.focus = crate::tui::Focus::Input;
                }
                ListDialogHit::SearchTitle => {
                    activate_search_for_dialog(&mut app.dialog_search, app.character_selected);
                }
                ListDialogHit::Inside => {}
            }
        }
        crate::tui::Focus::PersonaDialog => {
            let indices = super::filter_indices(&app.persona_names, &app.dialog_search);
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                LIST_DIALOG_TALL_PADDING,
                LIST_DIALOG_WIDTH,
            );
            match hit_test_list_dialog(
                dialog,
                &indices,
                app.persona_selected,
                mouse.column,
                mouse.row,
                Some(&app.dialog_search),
            ) {
                ListDialogHit::Item(i) => app.persona_selected = i,
                ListDialogHit::Outside => {
                    app.dialog_search.commit();
                    app.focus = crate::tui::Focus::Input;
                }
                ListDialogHit::SearchTitle => {
                    activate_search_for_dialog(&mut app.dialog_search, app.persona_selected);
                }
                ListDialogHit::Inside => {}
            }
        }
        crate::tui::Focus::SystemPromptDialog => {
            let indices = super::filter_indices(&app.system_prompt_list, &app.dialog_search);
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                LIST_DIALOG_TALL_PADDING,
                LIST_DIALOG_WIDTH,
            );
            match hit_test_list_dialog(
                dialog,
                &indices,
                app.system_prompt_selected,
                mouse.column,
                mouse.row,
                Some(&app.dialog_search),
            ) {
                ListDialogHit::Item(i) => app.system_prompt_selected = i,
                ListDialogHit::Outside => {
                    app.dialog_search.commit();
                    app.focus = app.system_editor_return_focus;
                }
                ListDialogHit::SearchTitle => {
                    activate_search_for_dialog(&mut app.dialog_search, app.system_prompt_selected);
                }
                ListDialogHit::Inside => {}
            }
        }
        crate::tui::Focus::BranchDialog => {
            let labels: Vec<String> = app
                .branch_dialog_items
                .iter()
                .map(|(_, label)| label.clone())
                .collect();
            let indices = super::filter_indices(&labels, &app.dialog_search);
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                super::FIELD_DIALOG_PADDING_ROWS,
                (terminal_area.width as f32 * DIALOG_WIDTH_RATIO) as u16,
            );
            match hit_test_list_dialog(
                dialog,
                &indices,
                app.branch_dialog_selected,
                mouse.column,
                mouse.row,
                Some(&app.dialog_search),
            ) {
                ListDialogHit::Item(i) => app.branch_dialog_selected = i,
                ListDialogHit::Outside => {
                    app.dialog_search.commit();
                    app.focus = crate::tui::Focus::Input;
                }
                ListDialogHit::SearchTitle => {
                    activate_search_for_dialog(&mut app.dialog_search, app.branch_dialog_selected);
                }
                ListDialogHit::Inside => {}
            }
        }
        crate::tui::Focus::WorldbookDialog => {
            let indices = super::filter_indices(&app.worldbook_list, &app.dialog_search);
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                LIST_DIALOG_TALL_PADDING,
                LIST_DIALOG_WIDTH,
            );
            match hit_test_list_dialog(
                dialog,
                &indices,
                app.worldbook_selected,
                mouse.column,
                mouse.row,
                Some(&app.dialog_search),
            ) {
                ListDialogHit::Item(i) => app.worldbook_selected = i,
                ListDialogHit::Outside => {
                    app.dialog_search.commit();
                    app.focus = crate::tui::Focus::Input;
                }
                ListDialogHit::SearchTitle => {
                    activate_search_for_dialog(&mut app.dialog_search, app.worldbook_selected);
                }
                ListDialogHit::Inside => {}
            }
        }
        crate::tui::Focus::PresetPickerDialog => {
            let indices = super::filter_indices(&app.preset_picker_names, &app.dialog_search);
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                LIST_DIALOG_TALL_PADDING,
                LIST_DIALOG_WIDTH,
            );
            match hit_test_list_dialog(
                dialog,
                &indices,
                app.preset_picker_selected,
                mouse.column,
                mouse.row,
                Some(&app.dialog_search),
            ) {
                ListDialogHit::Item(i) => app.preset_picker_selected = i,
                ListDialogHit::Outside => {
                    app.dialog_search.commit();
                    app.focus = crate::tui::Focus::ConfigDialog;
                }
                ListDialogHit::SearchTitle => {
                    activate_search_for_dialog(&mut app.dialog_search, app.preset_picker_selected);
                }
                ListDialogHit::Inside => {}
            }
        }
        crate::tui::Focus::DeleteConfirmDialog => {
            let dialog = crate::tui::render::centered_rect(LIST_DIALOG_WIDTH, 6, terminal_area);
            let pos = Position::new(mouse.column, mouse.row);
            if !dialog.contains(pos) {
                app.focus = crate::tui::Focus::Input;
            } else {
                let mid = dialog.x + dialog.width / 2;
                if mouse.column < mid {
                    app.delete_confirm_selected = 0;
                } else {
                    app.delete_confirm_selected = 1;
                }
            }
        }
        crate::tui::Focus::ConfigDialog => {
            if let Some(ref mut d) = app.config_dialog
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row) {
                    app.focus = crate::tui::Focus::Input;
                }
        }
        crate::tui::Focus::ThemeDialog => {
            if let Some(ref mut d) = app.theme_dialog
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row) {
                    app.focus = crate::tui::Focus::Input;
                }
        }
        crate::tui::Focus::BaseThemePickerDialog => {
            let indices: Vec<usize> = (0..app.base_theme_picker_names.len()).collect();
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                LIST_DIALOG_TALL_PADDING,
                LIST_DIALOG_WIDTH,
            );
            match hit_test_list_dialog(
                dialog,
                &indices,
                app.base_theme_picker_selected,
                mouse.column,
                mouse.row,
                None,
            ) {
                ListDialogHit::Item(i) => app.base_theme_picker_selected = i,
                ListDialogHit::Outside => app.focus = crate::tui::Focus::ThemeDialog,
                ListDialogHit::Inside | ListDialogHit::SearchTitle => {}
            }
        }
        crate::tui::Focus::PresetEditorDialog => {
            if let Some(ref mut d) = app.preset_editor
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row) {
                    app.focus = crate::tui::Focus::ConfigDialog;
                }
        }
        crate::tui::Focus::PersonaEditorDialog => {
            if let Some(ref mut d) = app.persona_editor
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row) {
                    app.focus = crate::tui::Focus::PersonaDialog;
                }
        }
        crate::tui::Focus::CharacterEditorDialog => {
            if let Some(ref mut d) = app.character_editor
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row) {
                    app.focus = crate::tui::Focus::CharacterDialog;
                }
        }
        crate::tui::Focus::SystemPromptEditorDialog => {
            if let Some(ref mut d) = app.system_prompt_editor
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row) {
                    app.focus = crate::tui::Focus::SystemPromptDialog;
                }
        }
        crate::tui::Focus::WorldbookEntryEditorDialog => {
            if let Some(ref mut d) = app.worldbook_entry_editor
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row) {
                    app.focus = crate::tui::Focus::WorldbookEditorDialog;
                }
        }
        crate::tui::Focus::WorldbookEditorDialog => {
            let entry_labels: Vec<String> = app
                .worldbook_editor_entries
                .iter()
                .map(|entry| {
                    let enabled = if entry.enabled { "+" } else { "-" };
                    let keys_str = if entry.keys.is_empty() {
                        "(no keys)".to_owned()
                    } else {
                        entry.keys.join(", ")
                    };
                    format!("[{enabled}] {keys_str}")
                })
                .collect();
            let indices = super::filter_indices(&entry_labels, &app.dialog_search);
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                LIST_DIALOG_TALL_PADDING + 2,
                FIELD_DIALOG_DEFAULT_WIDTH,
            );
            let pos = Position::new(mouse.column, mouse.row);
            if !dialog.contains(pos) {
                app.dialog_search.commit();
                app.focus = crate::tui::Focus::WorldbookDialog;
            } else if mouse.row + 1 == dialog.y + dialog.height
                && hit_search_region(&app.dialog_search, dialog, mouse.column)
            {
                if !app.dialog_search.active {
                    app.worldbook_editor_name_selected = false;
                    app.dialog_search.enter(app.worldbook_editor_selected);
                }
            } else if mouse.row == dialog.y + 1 {
                app.worldbook_editor_name_selected = true;
            } else {
                let items_area = Rect {
                    x: dialog.x + 1,
                    y: dialog.y + 3,
                    width: dialog.width.saturating_sub(2),
                    height: dialog.height.saturating_sub(4),
                };
                if let Some(entry_idx) = super::map_list_click(
                    items_area,
                    &indices,
                    app.worldbook_editor_selected,
                    mouse.row,
                ) {
                    app.worldbook_editor_name_selected = false;
                    app.worldbook_editor_selected = entry_idx;
                }
            }
        }
        crate::tui::Focus::WorldbookEntryDeleteDialog => {
            let dialog = crate::tui::render::centered_rect(LIST_DIALOG_WIDTH, 6, terminal_area);
            let pos = Position::new(mouse.column, mouse.row);
            if !dialog.contains(pos) {
                app.focus = crate::tui::Focus::WorldbookEditorDialog;
            }
        }
        crate::tui::Focus::EditDialog => {
            if let Some(ref mut editor) = app.edit_editor {
                let width = (terminal_area.width as f32 * DIALOG_WIDTH_RATIO) as u16;
                let height = (terminal_area.height as f32 * DIALOG_HEIGHT_RATIO) as u16;
                let dialog = crate::tui::render::centered_rect(width, height, terminal_area);
                let editor_area = Rect {
                    x: dialog.x + 2,
                    y: dialog.y + 1,
                    width: dialog.width.saturating_sub(4),
                    height: dialog.height.saturating_sub(2),
                };
                editor.cancel_selection();
                crate::tui::events::move_textarea_cursor_to_mouse(editor, editor_area, mouse.column, mouse.row);
            }
        }
        _ => {}
    }
}
