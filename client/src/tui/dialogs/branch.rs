//! Branch navigation dialog for browsing and switching conversation branches.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::ListItem;

use super::{clear_centered, render_hints_below_dialog};
use crate::tui::{Action, App, Focus};

pub(in crate::tui) fn render_branch_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let count = app.branch_dialog_items.len();
    let height = super::paged_list_height(count, area.height, super::FIELD_DIALOG_PADDING_ROWS);
    let width = (area.width as f32 * super::DIALOG_WIDTH_RATIO) as u16;
    let dialog = clear_centered(f, width, height, area);

    let items: Vec<ListItem<'_>> = app
        .branch_dialog_items
        .iter()
        .map(|(_node_id, label)| ListItem::new(label.clone()))
        .collect();

    super::render_paged_list(
        f,
        dialog,
        app.branch_dialog_selected,
        items,
        " Select Branch ",
        &app.theme,
    );

    render_hints_below_dialog(
        f,
        dialog,
        area,
        &[
            Line::from("Up/Down: navigate  PgUp/PgDn: page  Home/End: jump"),
            Line::from("Enter: select  Esc: cancel"),
        ],
    );
}

pub(in crate::tui) fn handle_branch_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.branch_dialog_items.is_empty() {
        if key.code == KeyCode::Esc {
            app.focus = Focus::Input;
        }
        return None;
    }

    let visible = super::page_size(app.last_terminal_height, super::FIELD_DIALOG_PADDING_ROWS);
    if super::handle_paged_list_key(
        &mut app.branch_dialog_selected,
        app.branch_dialog_items.len(),
        visible,
        key,
    ) == super::PagedListAction::Consumed
    {
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
