use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use super::{clear_centered, dialog_block};
use crate::tui::{Action, App, Focus};

pub(in crate::tui) fn render_branch_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let count = app.branch_dialog_items.len();
    let dialog = clear_centered(f, (area.width as f32 * 0.7) as u16, count as u16 + 4, area);

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

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Up/Down: navigate  Enter: select  Esc: cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let paragraph = Paragraph::new(Text::from(lines)).block(dialog_block(" Select Branch ", Color::Yellow));

    f.render_widget(paragraph, dialog);
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
            app.branch_dialog_selected = app.branch_dialog_selected.saturating_sub(1);
        }
        KeyCode::Down => {
            app.branch_dialog_selected =
                (app.branch_dialog_selected + 1).min(app.branch_dialog_items.len() - 1);
        }
        KeyCode::Enter => {
            let (node_id, _) = app.branch_dialog_items[app.branch_dialog_selected];
            app.session.tree.switch_to(node_id);
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
