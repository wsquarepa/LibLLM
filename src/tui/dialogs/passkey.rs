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

            tokio::spawn(async move {
                let event = match tokio::task::spawn_blocking(move || {
                    let salt_path = crate::config::salt_path();
                    let check_path = crate::config::key_check_path();
                    let result = crate::crypto::load_or_create_salt(&salt_path)
                        .and_then(|salt| crate::crypto::derive_key(&passkey, &salt));
                    match result {
                        Ok(derived_key) => {
                            match crate::crypto::verify_or_set_key(&check_path, &derived_key) {
                                Ok(true) => {
                                    let key = std::sync::Arc::new(derived_key);
                                    BackgroundEvent::KeyDerived(key, path)
                                }
                                Ok(false) => {
                                    BackgroundEvent::KeyDeriveFailed("Wrong passkey.".to_owned())
                                }
                                Err(err) => BackgroundEvent::KeyDeriveFailed(err.to_string()),
                            }
                        }
                        Err(err) => BackgroundEvent::KeyDeriveFailed(err.to_string()),
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
