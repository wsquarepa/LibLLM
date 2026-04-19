//! Branch navigation dialog for browsing and switching conversation branches.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::ListItem;

use super::{clear_centered, render_hints_below_dialog};
use crate::tui::{Action, App, Focus};

pub(in crate::tui) fn render_branch_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let labels: Vec<String> = app
        .branch_dialog_items
        .iter()
        .map(|(_, label)| label.clone())
        .collect();
    let visible_indices = super::filter_indices(&labels, &app.dialog_search);
    let unfiltered_total = labels.len();
    let count = visible_indices.len();
    let height = super::paged_list_height(count, area.height, super::FIELD_DIALOG_PADDING_ROWS);
    let width = (area.width as f32 * super::DIALOG_WIDTH_RATIO) as u16;
    let dialog = clear_centered(f, width, height, area);

    let filtered_selected = visible_indices
        .iter()
        .position(|&i| i == app.branch_dialog_selected)
        .unwrap_or(0);

    let items: Vec<ListItem<'_>> = visible_indices
        .iter()
        .map(|&i| ListItem::new(app.branch_dialog_items[i].1.clone()))
        .collect();

    super::render_paged_list(
        f,
        dialog,
        &app.theme,
        super::PagedListContent {
            selected: filtered_selected,
            items,
            title_base: " Select Branch ",
            search: Some(&app.dialog_search),
            unfiltered_total: Some(unfiltered_total),
        },
    );

    let hints = if app.dialog_search.active {
        vec![Line::from("Enter: apply  Esc: cancel  type to filter")]
    } else {
        vec![
            Line::from("Up/Down: navigate  PgUp/PgDn: page  Home/End: jump"),
            Line::from("Enter: select  Ctrl+F: search  Esc: cancel"),
        ]
    };
    render_hints_below_dialog(f, dialog, area, &hints);
}

pub(in crate::tui) fn handle_branch_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    let labels: Vec<String> = app
        .branch_dialog_items
        .iter()
        .map(|(_, label)| label.clone())
        .collect();

    if labels.is_empty() && !app.dialog_search.active {
        if key.code == KeyCode::Esc {
            app.focus = Focus::Input;
        }
        return None;
    }

    let visible = super::page_size(app.last_terminal_height, super::FIELD_DIALOG_PADDING_ROWS);
    let action = super::handle_paged_list_key(
        &mut app.branch_dialog_selected,
        &labels,
        visible,
        key,
        Some(&mut app.dialog_search),
    );
    if matches!(action, super::PagedListAction::Consumed | super::PagedListAction::EnteredSearch | super::PagedListAction::ExitedSearch) {
        return None;
    }

    match key.code {
        KeyCode::Enter => {
            let (node_id, _) = app.branch_dialog_items[app.branch_dialog_selected];
            app.session.tree.switch_to(node_id);
            app.invalidate_chat_cache();
            app.nav_cursor = None;
            app.auto_scroll = true;
            app.focus = Focus::Input;
            app.mark_session_dirty(super::super::SaveTrigger::Debounced, false);
            app.set_status(
                "Switched branch.".to_owned(),
                super::super::StatusLevel::Info,
            );
        }
        KeyCode::Esc => {
            app.focus = Focus::Input;
        }
        _ => {}
    }
    None
}
