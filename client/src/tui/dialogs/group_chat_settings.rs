//! Group-chat settings sheet: per-character talkativeness sliders, policy, card-assembly.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::types::Action;
use crate::tui::App;

pub(in crate::tui) fn render(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let chars = &app.session.characters;
    let policy_idx = chars.len();
    let assembly_idx = policy_idx + 1;
    let total_rows = assembly_idx + 1;

    let content_height = total_rows as u16 + 2;
    let width = (area.width as f32 * 0.55) as u16;
    let dialog = super::clear_centered(f, width, content_height, area);

    let mut lines: Vec<Line> = vec![Line::from("")];

    let notches_total = libllm::group_chat::TALKATIVENESS_NOTCHES as usize;
    let weights = libllm::group_chat::normalized_talkativeness(chars);
    for (idx, c) in chars.iter().enumerate() {
        let filled = libllm::group_chat::talkativeness_to_notch(c.talkativeness) as usize;
        let bar: String = "#".repeat(filled) + &".".repeat(notches_total - filled);
        let percent = (weights.get(idx).copied().unwrap_or(0.0) * 100.0).round() as u32;
        let row = format!(
            "  {:<16} [{bar}] {}/{notches_total}  ({percent:>3}%)",
            c.slug, filled,
        );
        let style = if app.group_settings_selected == idx {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(row, style)));
    }

    let policy_str = match app.session.chat_policy {
        libllm::group_chat::ChatPolicy::RoundRobin => "round-robin",
        libllm::group_chat::ChatPolicy::WeightedRandom => "weighted-random",
    };
    let assembly_str = match app.session.card_assembly {
        libllm::group_chat::CardAssembly::JoinCards => "join",
        libllm::group_chat::CardAssembly::SwapCards => "swap",
    };

    let policy_style = if app.group_settings_selected == policy_idx {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let assembly_style = if app.group_settings_selected == assembly_idx {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    lines.push(Line::from(Span::styled(
        format!("  policy:   {policy_str}"),
        policy_style,
    )));
    lines.push(Line::from(Span::styled(
        format!("  assembly: {assembly_str}"),
        assembly_style,
    )));

    let paragraph = Paragraph::new(lines)
        .block(crate::tui::render::dialog_block(" Group Chat Settings ", Color::Yellow));
    f.render_widget(paragraph, dialog);

    super::render_hints_below_dialog(
        f,
        dialog,
        area,
        &[Line::from(
            "Up/Down: navigate  Left/Right: adjust  Enter: save  Esc: cancel",
        )],
    );
}

pub(in crate::tui) fn handle_key(key: KeyEvent, app: &mut App) -> Option<Action> {
    let char_count = app.session.characters.len();
    let policy_idx = char_count;
    let assembly_idx = policy_idx + 1;
    let total_rows = assembly_idx + 1;

    match key.code {
        KeyCode::Esc => {
            crate::tui::dialog_handler::return_to_input(app);
            None
        }
        KeyCode::Up => {
            if app.group_settings_selected > 0 {
                app.group_settings_selected -= 1;
            }
            None
        }
        KeyCode::Down => {
            if app.group_settings_selected + 1 < total_rows {
                app.group_settings_selected += 1;
            }
            None
        }
        KeyCode::Left => {
            adjust(app, -1);
            None
        }
        KeyCode::Right => {
            adjust(app, 1);
            None
        }
        KeyCode::Enter => Some(Action::SaveGroupChatSettings),
        _ => None,
    }
}

fn adjust(app: &mut App, notch_delta: i32) {
    let char_count = app.session.characters.len();
    let policy_idx = char_count;
    let assembly_idx = policy_idx + 1;
    let i = app.group_settings_selected;

    if i < policy_idx {
        let slug = app.session.characters[i].slug.clone();
        if app.cli_overrides.talkativeness.contains_key(&slug) {
            app.set_status(
                format!("talkativeness for {slug} is locked by --talkativeness CLI flag"),
                crate::tui::StatusLevel::Warning,
            );
            return;
        }
        let current = libllm::group_chat::talkativeness_to_notch(
            app.session.characters[i].talkativeness,
        ) as i32;
        let max = libllm::group_chat::TALKATIVENESS_NOTCHES as i32;
        let new_notch = (current + notch_delta).clamp(0, max) as u8;
        let new_talk = libllm::group_chat::notch_to_talkativeness(new_notch);
        app.session.characters[i].talkativeness = new_talk;
        app.session.characters[i].action_points =
            libllm::group_chat::base_action_value(new_talk);
    } else if i == policy_idx {
        if app.cli_overrides.chat_policy.is_some() {
            app.set_status(
                "chat policy is locked by --chat-policy CLI flag".to_owned(),
                crate::tui::StatusLevel::Warning,
            );
            return;
        }
        app.session.chat_policy = match app.session.chat_policy {
            libllm::group_chat::ChatPolicy::RoundRobin => {
                libllm::group_chat::ChatPolicy::WeightedRandom
            }
            libllm::group_chat::ChatPolicy::WeightedRandom => {
                libllm::group_chat::ChatPolicy::RoundRobin
            }
        };
    } else if i == assembly_idx {
        if app.cli_overrides.card_assembly.is_some() {
            app.set_status(
                "card assembly is locked by --card-assembly CLI flag".to_owned(),
                crate::tui::StatusLevel::Warning,
            );
            return;
        }
        app.session.card_assembly = match app.session.card_assembly {
            libllm::group_chat::CardAssembly::JoinCards => {
                libllm::group_chat::CardAssembly::SwapCards
            }
            libllm::group_chat::CardAssembly::SwapCards => {
                libllm::group_chat::CardAssembly::JoinCards
            }
        };
    }
}
