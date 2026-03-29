use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use tokio::sync::mpsc;

use super::centered_rect;
use crate::tui::{Action, App, BackgroundEvent};

pub(in crate::tui) fn render_passkey_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let dialog = centered_rect(50, 7, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let masked: String = "*".repeat(app.passkey_input.len());
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Passkey: "),
            Span::styled(&masked, Style::default().fg(Color::Cyan)),
            if app.passkey_deriving {
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

    if app.passkey_deriving {
        lines.push(Line::from(Span::styled(
            "  Deriving key...",
            Style::default().fg(Color::Yellow),
        )));
    } else if !app.passkey_error.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", app.passkey_error),
            Style::default().fg(Color::Red),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  Enter to submit, Esc to quit",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Unlock Sessions ")
            .border_style(Style::default().fg(Color::Yellow)),
    );

    f.render_widget(paragraph, dialog);
}

pub(in crate::tui) fn handle_passkey_key(
    key: KeyEvent,
    app: &mut App,
    bg_tx: mpsc::Sender<BackgroundEvent>,
) -> Option<Action> {
    if app.passkey_deriving {
        return None;
    }
    match key.code {
        KeyCode::Enter => {
            let passkey = app.passkey_input.clone();
            let path = match &app.save_mode {
                crate::session::SaveMode::PendingPasskey(p) => p.clone(),
                _ => return None,
            };
            app.passkey_input.clear();
            app.passkey_error.clear();
            app.passkey_deriving = true;
            #[cfg(debug_assertions)]
            {
                app.unlock_debug = Some(crate::tui::UnlockDebugState {
                    kind: "unlock",
                    started_at: std::time::Instant::now(),
                });
            }

            tokio::spawn(async move {
                let event = match tokio::task::spawn_blocking(move || {
                    let total_start = std::time::Instant::now();
                    let salt_path = crate::config::salt_path();
                    let check_path = crate::config::key_check_path();
                    let salt_start = std::time::Instant::now();
                    let salt_result = crate::crypto::load_or_create_salt(&salt_path);
                    crate::debug_log::log_kv(
                        "unlock.phase",
                        &[
                            crate::debug_log::field("kind", "unlock"),
                            crate::debug_log::field("phase", "salt"),
                            crate::debug_log::field(
                                "result",
                                if salt_result.is_ok() { "ok" } else { "error" },
                            ),
                            crate::debug_log::field(
                                "elapsed_ms",
                                format!("{:.3}", salt_start.elapsed().as_secs_f64() * 1000.0),
                            ),
                            crate::debug_log::field("path", salt_path.display()),
                        ],
                    );
                    match salt_result {
                        Ok(salt) => {
                            let derive_start = std::time::Instant::now();
                            let derive_result = crate::crypto::derive_key(&passkey, &salt);
                            crate::debug_log::log_kv(
                                "unlock.phase",
                                &[
                                    crate::debug_log::field("kind", "unlock"),
                                    crate::debug_log::field("phase", "argon2"),
                                    crate::debug_log::field(
                                        "result",
                                        if derive_result.is_ok() { "ok" } else { "error" },
                                    ),
                                    crate::debug_log::field(
                                        "elapsed_ms",
                                        format!(
                                            "{:.3}",
                                            derive_start.elapsed().as_secs_f64() * 1000.0
                                        ),
                                    ),
                                ],
                            );
                            match derive_result {
                                Ok(derived_key) => {
                                    let verify_start = std::time::Instant::now();
                                    let verify_result =
                                        crate::crypto::verify_or_set_key(&check_path, &derived_key);
                                    let verify_status = match verify_result {
                                        Ok(true) => "ok",
                                        Ok(false) => "wrong_passkey",
                                        Err(_) => "error",
                                    };
                                    crate::debug_log::log_kv(
                                        "unlock.phase",
                                        &[
                                            crate::debug_log::field("kind", "unlock"),
                                            crate::debug_log::field("phase", "verify"),
                                            crate::debug_log::field("result", verify_status),
                                            crate::debug_log::field(
                                                "elapsed_ms",
                                                format!(
                                                    "{:.3}",
                                                    verify_start.elapsed().as_secs_f64() * 1000.0
                                                ),
                                            ),
                                            crate::debug_log::field("path", check_path.display()),
                                        ],
                                    );
                                    crate::debug_log::log_kv(
                                        "unlock.phase",
                                        &[
                                            crate::debug_log::field("kind", "unlock"),
                                            crate::debug_log::field("phase", "blocking_total"),
                                            crate::debug_log::field("result", verify_status),
                                            crate::debug_log::field(
                                                "elapsed_ms",
                                                format!(
                                                    "{:.3}",
                                                    total_start.elapsed().as_secs_f64() * 1000.0
                                                ),
                                            ),
                                        ],
                                    );
                                    match verify_result {
                                        Ok(true) => {
                                            let key = std::sync::Arc::new(derived_key);
                                            BackgroundEvent::KeyDerived(key, path)
                                        }
                                        Ok(false) => BackgroundEvent::KeyDeriveFailed(
                                            "Wrong passkey.".to_owned(),
                                        ),
                                        Err(err) => {
                                            BackgroundEvent::KeyDeriveFailed(err.to_string())
                                        }
                                    }
                                }
                                Err(err) => {
                                    crate::debug_log::log_kv(
                                        "unlock.phase",
                                        &[
                                            crate::debug_log::field("kind", "unlock"),
                                            crate::debug_log::field("phase", "blocking_total"),
                                            crate::debug_log::field("result", "error"),
                                            crate::debug_log::field(
                                                "elapsed_ms",
                                                format!(
                                                    "{:.3}",
                                                    total_start.elapsed().as_secs_f64() * 1000.0
                                                ),
                                            ),
                                            crate::debug_log::field("error", &err),
                                        ],
                                    );
                                    BackgroundEvent::KeyDeriveFailed(err.to_string())
                                }
                            }
                        }
                        Err(err) => {
                            crate::debug_log::log_kv(
                                "unlock.phase",
                                &[
                                    crate::debug_log::field("kind", "unlock"),
                                    crate::debug_log::field("phase", "blocking_total"),
                                    crate::debug_log::field("result", "error"),
                                    crate::debug_log::field(
                                        "elapsed_ms",
                                        format!(
                                            "{:.3}",
                                            total_start.elapsed().as_secs_f64() * 1000.0
                                        ),
                                    ),
                                    crate::debug_log::field("error", &err),
                                ],
                            );
                            BackgroundEvent::KeyDeriveFailed(err.to_string())
                        }
                    }
                })
                .await
                {
                    Ok(event) => event,
                    Err(err) => BackgroundEvent::KeyDeriveFailed(format!(
                        "key derivation task failed: {err}"
                    )),
                };
                let _ = bg_tx.send(event).await;
            });
            None
        }
        KeyCode::Char(c) => {
            app.passkey_input.push(c);
            app.passkey_error.clear();
            None
        }
        KeyCode::Backspace => {
            app.passkey_input.pop();
            app.passkey_error.clear();
            None
        }
        KeyCode::Esc => Some(Action::Quit),
        _ => None,
    }
}
