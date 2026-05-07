//! Status bar renderer showing version info and temporary notifications.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::App;

pub fn render_status_bar(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let bg_style = Style::default()
        .fg(app.theme.status_bar_fg)
        .bg(app.theme.status_bar_bg);

    if let Some(msg) = &app.status_message
        && matches!(msg.level, super::super::StatusLevel::Error)
    {
        let style = Style::default()
            .fg(app.theme.status_error_fg)
            .bg(app.theme.status_error_bg);
        let paragraph = Paragraph::new(format!(" {} ", msg.text))
            .style(style)
            .alignment(Alignment::Center);
        f.render_widget(paragraph, area);
        return;
    }

    let left_style = if !app.api_available {
        Style::default()
            .fg(app.theme.api_unavailable)
            .bg(app.theme.status_bar_bg)
    } else {
        bg_style
    };

    let hints_text = "Tab: switch focus | Ctrl+C: quit ";

    let total_width = area.width as usize;
    if total_width < 20 {
        let paragraph = Paragraph::new(format!(" {}", crate::version::STATUS_BAR)).style(left_style);
        f.render_widget(paragraph, area);
        return;
    }

    let notification = app.status_message.as_ref().map(|msg| {
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(msg.created);
        let remaining = msg.expires.saturating_duration_since(now);
        let slide_dur = super::super::NOTIFICATION_SLIDE_DURATION.as_secs_f64();

        let progress = if elapsed.as_secs_f64() < slide_dur {
            elapsed.as_secs_f64() / slide_dur
        } else if remaining.as_secs_f64() < slide_dur {
            remaining.as_secs_f64() / slide_dur
        } else {
            1.0
        };

        let (fg, bg) = match msg.level {
            super::super::StatusLevel::Info => (app.theme.status_info_fg, app.theme.status_info_bg),
            super::super::StatusLevel::Warning => {
                (app.theme.status_warning_fg, app.theme.status_warning_bg)
            }
            super::super::StatusLevel::Error => unreachable!(),
        };

        (msg.text.as_str(), fg, bg, progress)
    });

    let right_spans = build_right_spans(hints_text, notification, total_width, bg_style);
    let right_width: usize = right_spans.iter().map(|s| s.content.len()).sum();

    let left_max = total_width.saturating_sub(right_width).saturating_sub(1);
    let left_spans = build_left_spans(app, left_style, left_max);
    let left_width: usize = left_spans.iter().map(|s| s.content.len()).sum();

    let right_area = Rect::new(
        area.x + (total_width - right_width) as u16,
        area.y,
        right_width as u16,
        1,
    );

    f.render_widget(Paragraph::new("").style(bg_style), area);
    f.render_widget(
        Paragraph::new(Line::from(left_spans)).style(left_style),
        Rect::new(area.x, area.y, left_width.min(left_max) as u16, 1),
    );
    f.render_widget(
        Paragraph::new(Line::from(right_spans)).style(bg_style),
        right_area,
    );
}

fn build_left_spans<'a>(app: &'a App, base_style: Style, max_len: usize) -> Vec<Span<'a>> {
    let version = format!(" {}", crate::version::STATUS_BAR);
    let mut spans: Vec<Span<'a>> = Vec::new();

    if app.session.characters.len() >= 2 {
        let policy = match app.session.chat_policy {
            libllm::group_chat::ChatPolicy::RoundRobin => "RR",
            libllm::group_chat::ChatPolicy::WeightedRandom => "WR",
        };
        let assembly = match app.session.card_assembly {
            libllm::group_chat::CardAssembly::JoinCards => "join",
            libllm::group_chat::CardAssembly::SwapCards => "swap",
        };
        let n = app.session.characters.len();
        let group_chip = format!("[{n} chars · {policy} · {assembly}] ");
        spans.push(Span::styled(group_chip, base_style));

        let broken = app
            .session
            .characters
            .iter()
            .filter(|c| !app.character_cards_cache.contains_key(&c.slug))
            .count();
        if broken > 0 {
            let badge = format!("[{broken} missing] ");
            spans.push(Span::styled(
                badge,
                Style::default().fg(Color::Red).bg(base_style.bg.unwrap_or(Color::Reset)),
            ));
        }
    }

    let used: usize = spans.iter().map(|s| s.content.len()).sum();
    let version_budget = max_len.saturating_sub(used);
    let truncated_version = truncate_str(&version, version_budget);
    spans.push(Span::styled(truncated_version, base_style));

    spans
}

fn build_right_spans<'a>(
    hints: &'a str,
    notification: Option<(&'a str, Color, Color, f64)>,
    max_width: usize,
    bar_style: Style,
) -> Vec<Span<'a>> {
    let Some((text, fg, bg, progress)) = notification else {
        return vec![Span::styled(hints, bar_style)];
    };

    let padded = format!(" {} ", text);
    let notif_full_width = padded.len();
    let visible_width = ((progress * notif_full_width as f64).round() as usize).min(max_width);

    if visible_width == 0 {
        return vec![Span::styled(hints, bar_style)];
    }

    let hints_width = max_width.saturating_sub(visible_width);
    let visible_hints = truncate_str(hints, hints_width);

    let visible_text: String = if visible_width >= padded.len() {
        format!("{:width$}", padded, width = visible_width)
    } else {
        padded[..padded.floor_char_boundary(visible_width)].to_owned()
    };

    let notif_style = Style::default().fg(fg).bg(bg);
    let mut spans = Vec::new();

    if !visible_hints.is_empty() {
        spans.push(Span::styled(visible_hints, bar_style));
    }

    spans.push(Span::styled(visible_text, notif_style));

    spans
}

fn truncate_str(s: &str, max_len: usize) -> String {
    s[..s.floor_char_boundary(max_len)].to_owned()
}
