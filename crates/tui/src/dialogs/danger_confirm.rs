//! Confirmation dialog for Danger tab items 1-6 (synchronous destructive ops).

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::types::{App, DangerOp, Focus, StatusLevel};

use super::delete_confirm::ConfirmResult;
use super::{clear_centered, dialog_block};

pub(crate) fn render_danger_confirm(f: &mut Frame, area: Rect, op: DangerOp, selected: usize) {
    let (title, body, confirm_label) = match op {
        DangerOp::ClearStores => (
            "Clear Stores",
            "This will clear all dismissed-template prompts.",
            "Clear",
        ),
        DangerOp::RegeneratePresets => (
            "Regenerate Presets",
            "This will overwrite the bundled built-in presets.",
            "Regenerate",
        ),
        DangerOp::PurgeChats => (
            "Purge Chats",
            "This will delete ALL chats from the database.",
            "Purge",
        ),
        DangerOp::PurgeCharacters => (
            "Purge Characters",
            "This will delete ALL characters from the database.",
            "Purge",
        ),
        DangerOp::PurgePersonas => (
            "Purge Personas",
            "This will delete ALL personas from the database.",
            "Purge",
        ),
        DangerOp::PurgeWorldbooks => (
            "Purge Worldbooks",
            "This will delete ALL worldbooks from the database.",
            "Purge",
        ),
        DangerOp::DestroyAll => unreachable!("DestroyAll uses typed-confirm dialog"),
    };

    let width = area.width.min(68);
    let height = 9;
    let popup = clear_centered(f, width, height, area);

    let title_span = Span::styled(
        format!("Confirm: {title}"),
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    );
    let block = dialog_block(title_span, Color::Red);
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let body_style = Style::default().fg(Color::Red);
    let cancel_style = if selected == 0 {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let confirm_style = if selected == 1 {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red)
    };

    let lines = vec![
        Line::from(""),
        Line::styled(format!("  {body}"), body_style),
        Line::styled("  This action cannot be undone.", body_style),
        Line::from(""),
        Line::from(vec![
            Span::raw("    "),
            Span::styled(" Cancel ", cancel_style),
            Span::raw("   "),
            Span::styled(format!(" {confirm_label} "), confirm_style),
        ]),
    ];
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Left), inner);
}

pub(crate) fn handle_dialog_key(key: KeyEvent, app: &mut App) -> Option<crate::types::Action> {
    let mut sel = app.danger.confirm_selected.unwrap_or(0);
    let r = super::delete_confirm::handle_confirm_key(key, &mut sel);
    app.danger.confirm_selected = Some(sel);
    match r {
        ConfirmResult::Pending => {}
        ConfirmResult::Cancelled => {
            app.danger.confirm_op = None;
            app.danger.confirm_selected = None;
            app.focus = Focus::ConfigDialog;
        }
        ConfirmResult::Confirmed => {
            if let Some(op) = app.danger.confirm_op.take() {
                match crate::commands::danger::dispatch_sync(app, op) {
                    Ok(summary) => crate::commands::danger::report_summary(app, op, &summary),
                    Err(err) => app.set_status(format!("Op failed: {err}"), StatusLevel::Error),
                }
            }
            app.danger.confirm_selected = None;
            app.focus = Focus::ConfigDialog;
        }
    }
    None
}
