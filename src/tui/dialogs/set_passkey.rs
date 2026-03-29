use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use tokio::sync::mpsc;

use super::centered_rect;
use crate::tui::{Action, App, BackgroundEvent};

pub(in crate::tui) fn render_set_passkey_dialog(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let dialog = centered_rect(50, 9, area);
    f.render_widget(ratatui::widgets::Clear, dialog);

    let title = if app.set_passkey_is_initial {
        " Set Passkey "
    } else {
        " Change Passkey "
    };

    let new_masked: String = "*".repeat(app.set_passkey_input.len());
    let confirm_masked: String = "*".repeat(app.set_passkey_confirm.len());

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

    let new_value_style = if app.set_passkey_active_field == 0 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let confirm_value_style = if app.set_passkey_active_field == 1 {
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
    } else {
        lines.push(Line::from(Span::styled(
            "  Tab: switch field  Enter: submit  Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Yellow)),
    );

    f.render_widget(paragraph, dialog);
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

            tokio::spawn(async move {
                let salt_path = crate::config::salt_path();
                let check_path = crate::config::key_check_path();
                let result = crate::crypto::load_or_create_salt(&salt_path)
                    .and_then(|salt| crate::crypto::derive_key(&passkey, &salt));
                match result {
                    Ok(derived_key) => {
                        if let Err(e) =
                            crate::crypto::set_key_fingerprint(&check_path, &derived_key)
                        {
                            let _ = bg_tx
                                .send(BackgroundEvent::PasskeySetFailed(e.to_string()))
                                .await;
                            return;
                        }
                        let key = std::sync::Arc::new(derived_key);
                        let _ = bg_tx.send(BackgroundEvent::PasskeySet(key)).await;
                    }
                    Err(e) => {
                        let _ = bg_tx
                            .send(BackgroundEvent::PasskeySetFailed(e.to_string()))
                            .await;
                    }
                }
            });
            None
        }
        KeyCode::Char(c) => {
            if app.set_passkey_active_field == 0 {
                app.set_passkey_input.push(c);
            } else {
                app.set_passkey_confirm.push(c);
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
