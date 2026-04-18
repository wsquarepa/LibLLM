//! Mouse hit-testing for dialog list items and field editor regions.

use crossterm::event::MouseEvent;
use ratatui::layout::{Position, Rect};

use super::{DIALOG_HEIGHT_RATIO, DIALOG_WIDTH_RATIO, FIELD_DIALOG_DEFAULT_WIDTH, LIST_DIALOG_TALL_PADDING, LIST_DIALOG_WIDTH};

pub(in crate::tui) enum ListDialogHit {
    Item(usize),
    Outside,
    Inside,
}

pub(in crate::tui) fn hit_test_list_dialog(
    terminal_area: Rect,
    item_count: usize,
    screen_col: u16,
    screen_row: u16,
) -> ListDialogHit {
    let dialog_height = item_count as u16 + LIST_DIALOG_TALL_PADDING;
    let dialog = crate::tui::render::centered_rect(LIST_DIALOG_WIDTH, dialog_height, terminal_area);
    let pos = Position::new(screen_col, screen_row);
    if !dialog.contains(pos) {
        return ListDialogHit::Outside;
    }
    let inner_row = screen_row.saturating_sub(dialog.y + 2);
    if inner_row < item_count as u16 {
        ListDialogHit::Item(inner_row as usize)
    } else {
        ListDialogHit::Inside
    }
}

pub(in crate::tui) fn handle_dialog_mouse_click(mouse: MouseEvent, app: &mut crate::tui::App) {
    let terminal_area = match crossterm::terminal::size() {
        Ok((w, h)) => Rect::new(0, 0, w, h),
        Err(_) => return,
    };

    match app.focus {
        crate::tui::Focus::CharacterDialog => {
            match hit_test_list_dialog(
                terminal_area,
                app.character_names.len(),
                mouse.column,
                mouse.row,
            ) {
                ListDialogHit::Item(i) => app.character_selected = i,
                ListDialogHit::Outside => app.focus = crate::tui::Focus::Input,
                ListDialogHit::Inside => {}
            }
        }
        crate::tui::Focus::PersonaDialog => {
            match hit_test_list_dialog(
                terminal_area,
                app.persona_names.len(),
                mouse.column,
                mouse.row,
            ) {
                ListDialogHit::Item(i) => app.persona_selected = i,
                ListDialogHit::Outside => app.focus = crate::tui::Focus::Input,
                ListDialogHit::Inside => {}
            }
        }
        crate::tui::Focus::SystemPromptDialog => {
            match hit_test_list_dialog(
                terminal_area,
                app.system_prompt_list.len(),
                mouse.column,
                mouse.row,
            ) {
                ListDialogHit::Item(i) => app.system_prompt_selected = i,
                ListDialogHit::Outside => {
                    app.focus = app.system_editor_return_focus;
                }
                ListDialogHit::Inside => {}
            }
        }
        crate::tui::Focus::BranchDialog => {
            match hit_test_list_dialog(
                terminal_area,
                app.branch_dialog_items.len(),
                mouse.column,
                mouse.row,
            ) {
                ListDialogHit::Item(i) => app.branch_dialog_selected = i,
                ListDialogHit::Outside => app.focus = crate::tui::Focus::Input,
                ListDialogHit::Inside => {}
            }
        }
        crate::tui::Focus::WorldbookDialog => {
            match hit_test_list_dialog(
                terminal_area,
                app.worldbook_list.len(),
                mouse.column,
                mouse.row,
            ) {
                ListDialogHit::Item(i) => app.worldbook_selected = i,
                ListDialogHit::Outside => app.focus = crate::tui::Focus::Input,
                ListDialogHit::Inside => {}
            }
        }
        crate::tui::Focus::PresetPickerDialog => {
            match hit_test_list_dialog(
                terminal_area,
                app.preset_picker_names.len(),
                mouse.column,
                mouse.row,
            ) {
                ListDialogHit::Item(i) => app.preset_picker_selected = i,
                ListDialogHit::Outside => app.focus = crate::tui::Focus::ConfigDialog,
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
            match hit_test_list_dialog(
                terminal_area,
                app.base_theme_picker_names.len(),
                mouse.column,
                mouse.row,
            ) {
                ListDialogHit::Item(i) => app.base_theme_picker_selected = i,
                ListDialogHit::Outside => app.focus = crate::tui::Focus::ThemeDialog,
                ListDialogHit::Inside => {}
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
            let count = app.worldbook_editor_entries.len();
            let dialog_height = count as u16 + LIST_DIALOG_TALL_PADDING + 2;
            let dialog = crate::tui::render::centered_rect(
                FIELD_DIALOG_DEFAULT_WIDTH,
                dialog_height,
                terminal_area,
            );
            let pos = Position::new(mouse.column, mouse.row);
            if !dialog.contains(pos) {
                app.focus = crate::tui::Focus::WorldbookDialog;
            } else {
                let inner_row = mouse.row.saturating_sub(dialog.y + 2);
                if inner_row == 0 {
                    app.worldbook_editor_name_selected = true;
                } else if inner_row >= 2 {
                    let entry_idx = (inner_row - 2) as usize;
                    if entry_idx < count {
                        app.worldbook_editor_name_selected = false;
                        app.worldbook_editor_selected = entry_idx;
                    }
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
