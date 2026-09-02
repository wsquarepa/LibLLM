//! Mouse hit-testing for dialog list items and field editor regions.

use crossterm::event::MouseEvent;
use ratatui::layout::{Position, Rect};

use super::SearchState;
use super::{
    DIALOG_HEIGHT_RATIO, DIALOG_WIDTH_RATIO, FIELD_DIALOG_DEFAULT_WIDTH, LIST_DIALOG_TALL_PADDING,
    LIST_DIALOG_WIDTH,
};

pub(crate) enum ListDialogHit {
    Item(usize),
    Outside,
    Inside,
    SearchTitle,
}

pub(crate) fn hit_test_list_dialog(
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

pub(crate) fn handle_dialog_mouse_click(mouse: MouseEvent, app: &mut crate::App) {
    let terminal_area = match crossterm::terminal::size() {
        Ok((w, h)) => Rect::new(0, 0, w, h),
        Err(_) => return,
    };

    match app.focus {
        crate::Focus::CharacterDialog => {
            let indices = super::filter_indices(&app.character.names, &app.dialog_search);
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                LIST_DIALOG_TALL_PADDING,
                LIST_DIALOG_WIDTH,
            );
            match hit_test_list_dialog(
                dialog,
                &indices,
                app.character.selected,
                mouse.column,
                mouse.row,
                Some(&app.dialog_search),
            ) {
                ListDialogHit::Item(i) => app.character.selected = i,
                ListDialogHit::Outside => {
                    app.dialog_search.deactivate_and_clear();
                    app.focus = crate::Focus::Input;
                }
                ListDialogHit::SearchTitle => {
                    activate_search_for_dialog(&mut app.dialog_search, app.character.selected);
                }
                ListDialogHit::Inside => {}
            }
        }
        crate::Focus::PersonaDialog => {
            let indices = super::filter_indices(&app.persona.names, &app.dialog_search);
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                LIST_DIALOG_TALL_PADDING,
                LIST_DIALOG_WIDTH,
            );
            match hit_test_list_dialog(
                dialog,
                &indices,
                app.persona.selected,
                mouse.column,
                mouse.row,
                Some(&app.dialog_search),
            ) {
                ListDialogHit::Item(i) => app.persona.selected = i,
                ListDialogHit::Outside => {
                    app.dialog_search.deactivate_and_clear();
                    app.focus = crate::Focus::Input;
                }
                ListDialogHit::SearchTitle => {
                    activate_search_for_dialog(&mut app.dialog_search, app.persona.selected);
                }
                ListDialogHit::Inside => {}
            }
        }
        crate::Focus::SystemPromptDialog => {
            let indices = super::filter_indices(&app.system_prompt.list, &app.dialog_search);
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                LIST_DIALOG_TALL_PADDING,
                LIST_DIALOG_WIDTH,
            );
            match hit_test_list_dialog(
                dialog,
                &indices,
                app.system_prompt.selected,
                mouse.column,
                mouse.row,
                Some(&app.dialog_search),
            ) {
                ListDialogHit::Item(i) => app.system_prompt.selected = i,
                ListDialogHit::Outside => {
                    app.dialog_search.deactivate_and_clear();
                    app.focus = app.system_prompt.editor_return_focus;
                }
                ListDialogHit::SearchTitle => {
                    activate_search_for_dialog(&mut app.dialog_search, app.system_prompt.selected);
                }
                ListDialogHit::Inside => {}
            }
        }
        crate::Focus::BranchDialog => {
            let labels: Vec<String> = app
                .branch
                .items
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
                app.branch.selected,
                mouse.column,
                mouse.row,
                Some(&app.dialog_search),
            ) {
                ListDialogHit::Item(i) => app.branch.selected = i,
                ListDialogHit::Outside => {
                    app.dialog_search.deactivate_and_clear();
                    app.focus = crate::Focus::Input;
                }
                ListDialogHit::SearchTitle => {
                    activate_search_for_dialog(&mut app.dialog_search, app.branch.selected);
                }
                ListDialogHit::Inside => {}
            }
        }
        crate::Focus::WorldbookDialog => {
            let indices = super::filter_indices(&app.worldbook.list, &app.dialog_search);
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                LIST_DIALOG_TALL_PADDING,
                LIST_DIALOG_WIDTH,
            );
            match hit_test_list_dialog(
                dialog,
                &indices,
                app.worldbook.list_selected,
                mouse.column,
                mouse.row,
                Some(&app.dialog_search),
            ) {
                ListDialogHit::Item(i) => app.worldbook.list_selected = i,
                ListDialogHit::Outside => {
                    app.dialog_search.deactivate_and_clear();
                    app.focus = crate::Focus::Input;
                }
                ListDialogHit::SearchTitle => {
                    activate_search_for_dialog(&mut app.dialog_search, app.worldbook.list_selected);
                }
                ListDialogHit::Inside => {}
            }
        }
        crate::Focus::PresetPickerDialog => {
            let indices = super::filter_indices(&app.preset.picker_names, &app.dialog_search);
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                LIST_DIALOG_TALL_PADDING,
                LIST_DIALOG_WIDTH,
            );
            match hit_test_list_dialog(
                dialog,
                &indices,
                app.preset.picker_selected,
                mouse.column,
                mouse.row,
                Some(&app.dialog_search),
            ) {
                ListDialogHit::Item(i) => app.preset.picker_selected = i,
                ListDialogHit::Outside => {
                    app.dialog_search.deactivate_and_clear();
                    app.focus = crate::Focus::ConfigDialog;
                }
                ListDialogHit::SearchTitle => {
                    activate_search_for_dialog(&mut app.dialog_search, app.preset.picker_selected);
                }
                ListDialogHit::Inside => {}
            }
        }
        crate::Focus::DeleteConfirmDialog => {
            let dialog = crate::render::centered_rect(LIST_DIALOG_WIDTH, 6, terminal_area);
            let pos = Position::new(mouse.column, mouse.row);
            if !dialog.contains(pos) {
                app.focus = crate::Focus::Input;
            } else {
                let mid = dialog.x + dialog.width / 2;
                if mouse.column < mid {
                    app.delete_confirm.selected = 0;
                } else {
                    app.delete_confirm.selected = 1;
                }
            }
        }
        crate::Focus::ConfigDialog => {
            if let Some(ref mut d) = app.config_dialog
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row)
            {
                app.focus = crate::Focus::Input;
            }
        }
        crate::Focus::ThemeDialog => {
            if let Some(ref mut d) = app.theme_ui.dialog
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row)
            {
                app.focus = crate::Focus::Input;
            }
        }
        crate::Focus::BaseThemePickerDialog => {
            let indices: Vec<usize> = (0..app.theme_ui.base_picker_names.len()).collect();
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                LIST_DIALOG_TALL_PADDING,
                LIST_DIALOG_WIDTH,
            );
            match hit_test_list_dialog(
                dialog,
                &indices,
                app.theme_ui.base_picker_selected,
                mouse.column,
                mouse.row,
                None,
            ) {
                ListDialogHit::Item(i) => app.theme_ui.base_picker_selected = i,
                ListDialogHit::Outside => app.focus = crate::Focus::ThemeDialog,
                ListDialogHit::Inside | ListDialogHit::SearchTitle => {}
            }
        }
        crate::Focus::PresetEditorDialog => {
            if let Some(ref mut d) = app.preset.editor
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row)
            {
                app.focus = crate::Focus::ConfigDialog;
            }
        }
        crate::Focus::PersonaEditorDialog => {
            if let Some(ref mut d) = app.persona.editor
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row)
            {
                app.focus = crate::Focus::PersonaDialog;
            }
        }
        crate::Focus::AuthorNoteEditorDialog => {
            if let Some(ref mut d) = app.author_note_editor
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row)
            {
                app.focus = crate::Focus::Input;
            }
        }
        crate::Focus::CharacterEditorDialog => {
            if let Some(ref mut d) = app.character.editor
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row)
            {
                app.focus = crate::Focus::CharacterDialog;
            }
        }
        crate::Focus::SystemPromptEditorDialog => {
            if let Some(ref mut d) = app.system_prompt.editor
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row)
            {
                app.focus = crate::Focus::SystemPromptDialog;
            }
        }
        crate::Focus::WorldbookEntryEditorDialog => {
            if let Some(ref mut d) = app.worldbook.entry_editor
                && !d.handle_mouse_click(terminal_area, mouse.column, mouse.row)
            {
                app.focus = crate::Focus::WorldbookEditorDialog;
            }
        }
        crate::Focus::ScenarioEditorDialog => {
            crate::dialogs::scenario::handle_mouse_click(app, mouse.column, mouse.row);
        }
        crate::Focus::WorldbookEditorDialog => {
            let entry_labels = super::worldbook::editor_entry_labels(&app.worldbook.editor_entries);
            let indices = super::filter_indices(&entry_labels, &app.dialog_search);
            let dialog = super::list_dialog_rect(
                terminal_area,
                indices.len(),
                LIST_DIALOG_TALL_PADDING + 2,
                FIELD_DIALOG_DEFAULT_WIDTH,
            );
            let pos = Position::new(mouse.column, mouse.row);
            if !dialog.contains(pos) {
                app.dialog_search.deactivate_and_clear();
                app.focus = crate::Focus::WorldbookDialog;
            } else if mouse.row + 1 == dialog.y + dialog.height
                && hit_search_region(&app.dialog_search, dialog, mouse.column)
            {
                if !app.dialog_search.active {
                    app.worldbook.editor_name_selected = false;
                    app.dialog_search.enter(app.worldbook.editor_selected);
                }
            } else if mouse.row == dialog.y + 1 {
                app.worldbook.editor_name_selected = true;
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
                    app.worldbook.editor_selected,
                    mouse.row,
                ) {
                    app.worldbook.editor_name_selected = false;
                    app.worldbook.editor_selected = entry_idx;
                }
            }
        }
        crate::Focus::WorldbookEntryDeleteDialog => {
            let dialog = crate::render::centered_rect(LIST_DIALOG_WIDTH, 6, terminal_area);
            let pos = Position::new(mouse.column, mouse.row);
            if !dialog.contains(pos) {
                app.focus = crate::Focus::WorldbookEditorDialog;
            }
        }
        crate::Focus::EditDialog => {
            if let Some(ref mut editor) = app.edit.editor {
                let width = (terminal_area.width as f32 * DIALOG_WIDTH_RATIO) as u16;
                let height = (terminal_area.height as f32 * DIALOG_HEIGHT_RATIO) as u16;
                let dialog = crate::render::centered_rect(width, height, terminal_area);
                let editor_area = Rect {
                    x: dialog.x + 2,
                    y: dialog.y + 1,
                    width: dialog.width.saturating_sub(4),
                    height: dialog.height.saturating_sub(2),
                };
                editor.cancel_selection();
                crate::events::move_textarea_cursor_to_mouse(
                    editor,
                    editor_area,
                    app.edit.scroll_top,
                    mouse.column,
                    mouse.row,
                );
            }
        }
        _ => {}
    }
}
