use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::tui::{Action, App, Focus};

pub(in crate::tui) fn render_branch_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let count = app.branch_dialog_items.len();
    let dialog = clear_centered(
        f,
        (area.width as f32 * super::DIALOG_WIDTH_RATIO) as u16,
        count as u16 + super::FIELD_DIALOG_PADDING_ROWS,
        area,
    );

    let mut lines: Vec<Line> = vec![Line::from("")];

    for (i, (_node_id, label)) in app.branch_dialog_items.iter().enumerate() {
        let is_selected = i == app.branch_dialog_selected;
        let marker = if is_selected { "> " } else { "  " };
        let style = if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(format!("{marker}{label}"), style)));
    }

    let paragraph = Paragraph::new(Text::from(lines)).block(dialog_block(" Select Branch ", Color::Yellow));

    f.render_widget(paragraph, dialog);

    render_hints_below_dialog(f, dialog, area, &[
        Line::from("Up/Down: navigate  Enter: select  Esc: cancel"),
    ]);
}

pub(in crate::tui) fn handle_branch_dialog_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    if app.branch_dialog_items.is_empty() {
        if key.code == KeyCode::Esc {
            app.focus = Focus::Input;
        }
        return None;
    }

    match key.code {
        KeyCode::Up => {
            super::move_selection_up(&mut app.branch_dialog_selected);
        }
        KeyCode::Down => {
            super::move_selection_down(&mut app.branch_dialog_selected, app.branch_dialog_items.len());
        }
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
