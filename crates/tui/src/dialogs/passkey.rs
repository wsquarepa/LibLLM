//! Passkey entry dialog for unlocking an encrypted database on startup.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use tokio::sync::mpsc;

use super::{clear_centered, dialog_block, render_hints_below_dialog};
use crate::{Action, App, BackgroundEvent};

const DIALOG_WIDTH: u16 = 50;
const DIALOG_HEIGHT: u16 = 6;
const LABEL_PREFIX_LEN: usize = 11; // "  Passkey: "

pub(crate) fn render_passkey_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let dialog = clear_centered(f, DIALOG_WIDTH, DIALOG_HEIGHT, area);

    let max_visible = DIALOG_WIDTH as usize - 2 - LABEL_PREFIX_LEN - 1;
    let masked_full: String = "*".repeat(app.passkey.input.len());
    let masked: String = if masked_full.len() > max_visible {
        masked_full[masked_full.len() - max_visible..].to_owned()
    } else {
        masked_full
    };
    let passkey_color = if super::is_flash_active(app.input_reject_flash) {
        Color::Yellow
    } else {
        Color::Cyan
    };
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Passkey: "),
            Span::styled(&masked, Style::default().fg(passkey_color)),
            if app.passkey.deriving {
                Span::raw("")
            } else {
                Span::styled(
                    "_",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::SLOW_BLINK),
                )
            },
        ]),
        Line::from(""),
    ];

    if app.passkey.deriving {
        lines.push(Line::from(Span::styled(
            "  Deriving key...",
            Style::default().fg(Color::Yellow),
        )));
    } else if !app.passkey.error.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", app.passkey.error),
            Style::default().fg(Color::Red),
        )));
    }

    let paragraph =
        Paragraph::new(Text::from(lines)).block(dialog_block(" Unlock Sessions ", Color::Yellow));

    f.render_widget(paragraph, dialog);

    if !app.passkey.deriving && app.passkey.error.is_empty() {
        render_hints_below_dialog(
            f,
            dialog,
            area,
            &[Line::from("Enter to submit, Esc to quit")],
        );
    }
}

pub(crate) fn handle_passkey_key(
    key: KeyEvent,
    app: &mut App,
    bg_tx: mpsc::Sender<BackgroundEvent>,
) -> Option<Action> {
    if app.passkey.deriving {
        return None;
    }
    match key.code {
        KeyCode::Enter => {
            let passkey = app.passkey.input.clone();
            if !matches!(
                &app.save_mode,
                libllm_core::session::SaveMode::PendingPasskey { .. }
            ) {
                return None;
            }
            let db_path = libllm_config::data_dir().join("data.db");
            let salt_path = libllm_config::salt_path();
            app.passkey.resolved = Some(passkey.clone());
            app.passkey.input.clear();
            app.passkey.error.clear();
            app.passkey.deriving = true;
            app.unlock_debug = Some(crate::UnlockDebugState {
                kind: "unlock",
                started_at: std::time::Instant::now(),
            });

            tokio::spawn(async move {
                let event = match tokio::task::spawn_blocking(move || {
                    super::derive_key_blocking(salt_path, passkey, "unlock", |derived_key| {
                        let verify_start = std::time::Instant::now();
                        let verify_result =
                            libllm_storage::db::Database::open(&db_path, Some(&derived_key));
                        let verify_status = if verify_result.is_ok() {
                            "ok"
                        } else {
                            "wrong_passkey"
                        };
                        super::log_phase_with_path(
                            "unlock",
                            "verify",
                            verify_status,
                            verify_start.elapsed(),
                            db_path.display(),
                        );
                        match verify_result {
                            Ok(_db) => {
                                let key = std::sync::Arc::new(derived_key);
                                BackgroundEvent::KeyDerived(key, db_path)
                            }
                            Err(_) => BackgroundEvent::KeyDeriveFailed("Wrong passkey.".to_owned()),
                        }
                    })
                })
                .await
                {
                    Ok(event) => event,
                    Err(err) => BackgroundEvent::KeyDeriveFailed(format!(
                        "key derivation task failed: {err}"
                    )),
                };
                if let Err(err) = bg_tx.send(event).await {
                    tracing::error!(result = "error", error = %err, "tui.passkey.send_failed");
                }
            });
            None
        }
        KeyCode::Char(c) => {
            if app.passkey.input.len() < super::MAX_PASSKEY_LENGTH {
                app.passkey.input.push(c);
            } else {
                app.input_reject_flash = Some(std::time::Instant::now());
            }
            app.passkey.error.clear();
            None
        }
        KeyCode::Backspace => {
            app.passkey.input.pop();
            app.passkey.error.clear();
            None
        }
        KeyCode::Esc => Some(Action::Quit),
        _ => None,
    }
}
