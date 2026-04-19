//! Status bar renderer showing model info, token count, and temporary notifications.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::App;

pub fn render_status_bar(
    f: &mut ratatui::Frame,
    app: &App,
    area: Rect,
    branch_info: Option<(usize, usize)>,
    token_count: usize,
) {
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

    let branch_text = match branch_info {
        Some((idx, total)) => format!("Branch {}/{total}", idx + 1),
        None => "Linear".to_owned(),
    };

    let worldbook_text = if app.session.character.is_some() {
        let mut count = app.config.worldbooks.len();
        for name in &app.session.worldbooks {
            if !app.config.worldbooks.contains(name) {
                count += 1;
            }
        }
        format!(" | {count} worldbooks")
    } else {
        String::new()
    };

    let display_name = app.model_name.as_deref().unwrap_or("connecting...");
    let left_text = format!(
        " {} | {} | ~{} tokens | {}{}",
        display_name, app.instruct_preset.name, token_count, branch_text, worldbook_text,
    );

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
        let paragraph = Paragraph::new(left_text).style(left_style);
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
    let truncated_left = truncate_str(&left_text, left_max);

    let left_area = Rect::new(area.x, area.y, left_max as u16, 1);
    let right_area = Rect::new(
        area.x + (total_width - right_width) as u16,
        area.y,
        right_width as u16,
        1,
    );

    f.render_widget(Paragraph::new("").style(bg_style), area);
    f.render_widget(Paragraph::new(truncated_left).style(left_style), left_area);
    f.render_widget(
        Paragraph::new(Line::from(right_spans)).style(bg_style),
        right_area,
    );
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
