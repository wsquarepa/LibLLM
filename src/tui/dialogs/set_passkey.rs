use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use tokio::sync::mpsc;

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::tui::{Action, App, BackgroundEvent};

const DIALOG_WIDTH: u16 = 50;
const DIALOG_HEIGHT: u16 = 8;
const LABEL_PREFIX_LEN: usize = 19; // "  New Passkey:     "

pub(in crate::tui) fn render_set_passkey_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let dialog = clear_centered(f, DIALOG_WIDTH, DIALOG_HEIGHT, area);

    let title = if app.set_passkey_is_initial {
        " Set Passkey "
    } else {
        " Change Passkey "
    };

    let max_visible = DIALOG_WIDTH as usize - 2 - LABEL_PREFIX_LEN - 1;
    let new_masked_full: String = "*".repeat(app.set_passkey_input.len());
    let new_masked: String = if new_masked_full.len() > max_visible {
        new_masked_full[new_masked_full.len() - max_visible..].to_owned()
    } else {
        new_masked_full
    };
    let confirm_masked_full: String = "*".repeat(app.set_passkey_confirm.len());
    let confirm_masked: String = if confirm_masked_full.len() > max_visible {
        confirm_masked_full[confirm_masked_full.len() - max_visible..].to_owned()
    } else {
        confirm_masked_full
    };

    let new_label_style = if app.set_passkey_active_field == 0 {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let confirm_label_style = if app.set_passkey_active_field == 1 {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let flashing = super::is_flash_active(app.input_reject_flash);
    let new_value_style = if app.set_passkey_active_field == 0 && flashing {
        Style::default().fg(Color::Yellow)
    } else if app.set_passkey_active_field == 0 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let confirm_value_style = if app.set_passkey_active_field == 1 && flashing {
        Style::default().fg(Color::Yellow)
    } else if app.set_passkey_active_field == 1 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let cursor = |active: bool| -> Span {
        if active && !app.set_passkey_deriving {
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
            cursor(app.set_passkey_active_field == 0),
        ]),
        Line::from(vec![
            Span::styled("  Confirm:         ", confirm_label_style),
            Span::styled(&confirm_masked, confirm_value_style),
            cursor(app.set_passkey_active_field == 1),
        ]),
        Line::from(""),
    ];

    if app.set_passkey_deriving {
        lines.push(Line::from(Span::styled(
            "  Deriving key...",
            Style::default().fg(Color::Yellow),
        )));
    } else if !app.set_passkey_error.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", app.set_passkey_error),
            Style::default().fg(Color::Red),
        )));
    }

    let paragraph = Paragraph::new(Text::from(lines)).block(dialog_block(title, Color::Yellow));

    f.render_widget(paragraph, dialog);

    if !app.set_passkey_deriving && app.set_passkey_error.is_empty() {
        render_hints_below_dialog(f, dialog, area, &[
            Line::from("Tab: switch field  Enter: submit  Esc: cancel"),
        ]);
    }
}

pub(in crate::tui) fn handle_set_passkey_key(
    key: KeyEvent,
    app: &mut App,
    bg_tx: mpsc::Sender<BackgroundEvent>,
) -> Option<Action> {
    if app.set_passkey_deriving {
        return None;
    }
    match key.code {
        KeyCode::Tab | KeyCode::Up | KeyCode::Down | KeyCode::BackTab => {
            app.set_passkey_active_field = 1 - app.set_passkey_active_field;
            None
        }
        KeyCode::Enter => {
            if app.set_passkey_input.is_empty() {
                app.set_passkey_error = "Passkey cannot be empty".to_owned();
                return None;
            }
            if app.set_passkey_input != app.set_passkey_confirm {
                app.set_passkey_error = "Passkeys do not match".to_owned();
                return None;
            }

            let passkey = app.set_passkey_input.clone();
            app.set_passkey_input.clear();
            app.set_passkey_confirm.clear();
            app.set_passkey_error.clear();
            app.set_passkey_deriving = true;
            app.unlock_debug = Some(crate::tui::UnlockDebugState {
                kind: if app.set_passkey_is_initial {
                    "set_passkey"
                } else {
                    "change_passkey"
                },
                started_at: std::time::Instant::now(),
            });
            let is_initial = app.set_passkey_is_initial;
            let debug_kind = if is_initial {
                "set_passkey"
            } else {
                "change_passkey"
            };

            tokio::spawn(async move {
                let event = match tokio::task::spawn_blocking(move || {
                    super::derive_key_blocking(
                        passkey,
                        debug_kind,
                        |derived_key, check_path| {
                            if is_initial {
                                let fingerprint_start = std::time::Instant::now();
                                let fingerprint_result =
                                    crate::crypto::set_key_fingerprint(check_path, &derived_key);
                                let fingerprint_status = if fingerprint_result.is_ok() {
                                    "ok"
                                } else {
                                    "error"
                                };
                                super::log_phase_with_path(
                                    debug_kind,
                                    "fingerprint",
                                    fingerprint_status,
                                    fingerprint_start.elapsed(),
                                    check_path.display(),
                                );
                                match fingerprint_result {
                                    Ok(()) => {
                                        let key = std::sync::Arc::new(derived_key);
                                        BackgroundEvent::PasskeySet(key)
                                    }
                                    Err(err) => {
                                        BackgroundEvent::PasskeySetFailed(err.to_string())
                                    }
                                }
                            } else {
                                let key = std::sync::Arc::new(derived_key);
                                BackgroundEvent::PasskeySet(key)
                            }
                        },
                    )
                })
                .await
                {
                    Ok(event) => event,
                    Err(err) => {
                        BackgroundEvent::PasskeySetFailed(format!("passkey task failed: {err}"))
                    }
                };
                let _ = bg_tx.send(event).await;
            });
            None
        }
        KeyCode::Char(c) => {
            let rejected;
            if app.set_passkey_active_field == 0 {
                if app.set_passkey_input.len() < super::MAX_PASSKEY_LENGTH {
                    app.set_passkey_input.push(c);
                    rejected = false;
                } else {
                    rejected = true;
                }
            } else if app.set_passkey_confirm.len() < super::MAX_PASSKEY_LENGTH {
                app.set_passkey_confirm.push(c);
                rejected = false;
            } else {
                rejected = true;
            }
            if rejected {
                app.input_reject_flash = Some(std::time::Instant::now());
            }
            app.set_passkey_error.clear();
            None
        }
        KeyCode::Backspace => {
            if app.set_passkey_active_field == 0 {
                app.set_passkey_input.pop();
            } else {
                app.set_passkey_confirm.pop();
            }
            app.set_passkey_error.clear();
            None
        }
        KeyCode::Esc => {
            if app.set_passkey_is_initial {
                Some(Action::Quit)
            } else {
                app.set_passkey_input.clear();
                app.set_passkey_confirm.clear();
                app.set_passkey_error.clear();
                app.set_passkey_active_field = 0;
                app.focus = crate::tui::Focus::Input;
                None
            }
        }
        _ => None,
    }
}
