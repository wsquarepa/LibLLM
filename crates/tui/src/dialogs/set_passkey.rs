//! Set/change passkey dialog with confirmation field.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use tokio::sync::mpsc;

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::{Action, App, BackgroundEvent};

const DIALOG_WIDTH: u16 = 50;
const DIALOG_HEIGHT: u16 = 8;
const LABEL_PREFIX_LEN: usize = 19; // "  New Passkey:     "

pub(crate) fn render_set_passkey_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let dialog = clear_centered(f, DIALOG_WIDTH, DIALOG_HEIGHT, area);

    let title = if app.set_passkey.is_initial {
        " Set Passkey "
    } else {
        " Change Passkey "
    };

    let max_visible = DIALOG_WIDTH as usize - 2 - LABEL_PREFIX_LEN - 1;
    let new_masked = super::masked_and_truncated(app.set_passkey.input.len(), max_visible);
    let confirm_masked = super::masked_and_truncated(app.set_passkey.confirm.len(), max_visible);

    let new_label_style = if app.set_passkey.active_field == 0 {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let confirm_label_style = if app.set_passkey.active_field == 1 {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let flashing = super::is_flash_active(app.input_reject_flash);
    let new_value_style = if app.set_passkey.active_field == 0 && flashing {
        Style::default().fg(Color::Yellow)
    } else if app.set_passkey.active_field == 0 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let confirm_value_style = if app.set_passkey.active_field == 1 && flashing {
        Style::default().fg(Color::Yellow)
    } else if app.set_passkey.active_field == 1 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let cursor = |active: bool| -> Span {
        if active && !app.set_passkey.deriving {
            Span::styled(
                "_",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::SLOW_BLINK),
            )
        } else {
            Span::raw("")
        }
    };

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  New Passkey:     ", new_label_style),
            Span::styled(&new_masked, new_value_style),
            cursor(app.set_passkey.active_field == 0),
        ]),
        Line::from(vec![
            Span::styled("  Confirm:         ", confirm_label_style),
            Span::styled(&confirm_masked, confirm_value_style),
            cursor(app.set_passkey.active_field == 1),
        ]),
        Line::from(""),
    ];

    if app.set_passkey.deriving {
        lines.push(Line::from(Span::styled(
            "  Deriving key...",
            Style::default().fg(Color::Yellow),
        )));
    } else if !app.set_passkey.error.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", app.set_passkey.error),
            Style::default().fg(Color::Red),
        )));
    }

    let paragraph = Paragraph::new(Text::from(lines)).block(dialog_block(title, Color::Yellow));

    f.render_widget(paragraph, dialog);

    if !app.set_passkey.deriving && app.set_passkey.error.is_empty() {
        render_hints_below_dialog(
            f,
            dialog,
            area,
            &[Line::from("Tab: switch field  Enter: submit  Esc: cancel")],
        );
    }
}

pub(crate) fn handle_set_passkey_key(
    key: KeyEvent,
    app: &mut App,
    bg_tx: mpsc::Sender<BackgroundEvent>,
) -> Option<Action> {
    if app.set_passkey.deriving {
        return None;
    }
    match key.code {
        KeyCode::Tab | KeyCode::Up | KeyCode::Down | KeyCode::BackTab => {
            app.set_passkey.active_field = 1 - app.set_passkey.active_field;
            None
        }
        KeyCode::Enter => {
            if app.set_passkey.input.is_empty() {
                app.set_passkey.error = "Passkey cannot be empty".to_owned();
                return None;
            }
            if app.set_passkey.input != app.set_passkey.confirm {
                app.set_passkey.error = "Passkeys do not match".to_owned();
                return None;
            }

            let passkey = app.set_passkey.input.clone();
            if app.set_passkey.is_initial {
                app.passkey.resolved = Some(passkey.clone());
            } else {
                app.passkey.pending_new = Some(passkey.clone());
            }
            app.set_passkey.input.clear();
            app.set_passkey.confirm.clear();
            app.set_passkey.error.clear();
            app.set_passkey.deriving = true;
            app.unlock_debug = Some(crate::UnlockDebugState {
                kind: if app.set_passkey.is_initial {
                    "set_passkey"
                } else {
                    "change_passkey"
                },
                started_at: std::time::Instant::now(),
            });
            let is_initial = app.set_passkey.is_initial;
            let debug_kind = if is_initial {
                "set_passkey"
            } else {
                "change_passkey"
            };

            let salt_path = libllm_config::salt_path();

            tokio::spawn(async move {
                let event = match tokio::task::spawn_blocking(move || {
                    super::derive_key_blocking(salt_path, passkey, debug_kind, |derived_key| {
                        let key = std::sync::Arc::new(derived_key);
                        BackgroundEvent::PasskeySet(key)
                    })
                })
                .await
                {
                    Ok(event) => event,
                    Err(err) => {
                        BackgroundEvent::PasskeySetFailed(format!("passkey task failed: {err}"))
                    }
                };
                if let Err(err) = bg_tx.send(event).await {
                    tracing::error!(result = "error", error = %err, "tui.passkey.send_failed");
                }
            });
            None
        }
        KeyCode::Char(c) => {
            let rejected;
            if app.set_passkey.active_field == 0 {
                if app.set_passkey.input.len() < super::MAX_PASSKEY_LENGTH {
                    app.set_passkey.input.push(c);
                    rejected = false;
                } else {
                    rejected = true;
                }
            } else if app.set_passkey.confirm.len() < super::MAX_PASSKEY_LENGTH {
                app.set_passkey.confirm.push(c);
                rejected = false;
            } else {
                rejected = true;
            }
            if rejected {
                app.input_reject_flash = Some(std::time::Instant::now());
            }
            app.set_passkey.error.clear();
            None
        }
        KeyCode::Backspace => {
            if app.set_passkey.active_field == 0 {
                app.set_passkey.input.pop();
            } else {
                app.set_passkey.confirm.pop();
            }
            app.set_passkey.error.clear();
            None
        }
        KeyCode::Esc => {
            if app.set_passkey.is_initial {
                Some(Action::Quit)
            } else {
                app.set_passkey.input.clear();
                app.set_passkey.confirm.clear();
                app.set_passkey.error.clear();
                app.set_passkey.active_field = 0;
                app.focus = crate::Focus::Input;
                None
            }
        }
        _ => None,
    }
}
