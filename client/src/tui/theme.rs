//! Theme resolution and color palette definitions.

use ratatui::style::Color;

use libllm::config::{ColorLabel, Config, ThemeColorOverrides};

#[derive(PartialEq)]
pub struct Theme {
    pub user_message: Color,
    pub assistant_message_fg: Color,
    pub assistant_message_bg: Color,
    pub system_message: Color,
    pub border_focused: Color,
    pub border_unfocused: Color,
    pub status_bar_fg: Color,
    pub status_bar_bg: Color,
    pub status_error_fg: Color,
    pub status_error_bg: Color,
    pub status_info_fg: Color,
    pub status_info_bg: Color,
    pub status_warning_fg: Color,
    pub status_warning_bg: Color,
    pub dialogue: Color,
    pub nav_cursor_fg: Color,
    pub nav_cursor_bg: Color,
    pub hover_bg: Color,
    pub dimmed: Color,
    pub sidebar_highlight_fg: Color,
    pub sidebar_highlight_bg: Color,
    pub command_picker_fg: Color,
    pub command_picker_bg: Color,
    pub streaming_indicator: Color,
    pub api_unavailable: Color,
    pub summary_indicator: Color,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            user_message: Color::Green,
            assistant_message_fg: Color::White,
            assistant_message_bg: Color::Blue,
            system_message: Color::DarkGray,
            border_focused: Color::Cyan,
            border_unfocused: Color::DarkGray,
            status_bar_fg: Color::White,
            status_bar_bg: Color::DarkGray,
            status_error_fg: Color::White,
            status_error_bg: Color::Red,
            status_info_fg: Color::White,
            status_info_bg: Color::Blue,
            status_warning_fg: Color::Black,
            status_warning_bg: Color::Yellow,
            dialogue: Color::LightBlue,
            nav_cursor_fg: Color::Black,
            nav_cursor_bg: Color::Yellow,
            hover_bg: Color::Indexed(236),
            dimmed: Color::DarkGray,
            sidebar_highlight_fg: Color::Black,
            sidebar_highlight_bg: Color::Cyan,
            command_picker_fg: Color::Black,
            command_picker_bg: Color::Yellow,
            streaming_indicator: Color::Yellow,
            api_unavailable: Color::Red,
            summary_indicator: Color::DarkGray,
        }
    }

    pub fn light() -> Self {
        Self {
            user_message: Color::Blue,
            assistant_message_fg: Color::White,
            assistant_message_bg: Color::Magenta,
            system_message: Color::DarkGray,
            border_focused: Color::Blue,
            border_unfocused: Color::DarkGray,
            status_bar_fg: Color::White,
            status_bar_bg: Color::Indexed(238),
            status_error_fg: Color::White,
            status_error_bg: Color::Red,
            status_info_fg: Color::White,
            status_info_bg: Color::Blue,
            status_warning_fg: Color::Black,
            status_warning_bg: Color::Yellow,
            dialogue: Color::Magenta,
            nav_cursor_fg: Color::White,
            nav_cursor_bg: Color::Blue,
            hover_bg: Color::Indexed(254),
            dimmed: Color::DarkGray,
            sidebar_highlight_fg: Color::White,
            sidebar_highlight_bg: Color::Blue,
            command_picker_fg: Color::White,
            command_picker_bg: Color::Blue,
            streaming_indicator: Color::Blue,
            api_unavailable: Color::Red,
            summary_indicator: Color::Gray,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "dark" => Some(Self::dark()),
            "light" => Some(Self::light()),
            _ => None,
        }
    }

    pub fn apply_overrides(&mut self, overrides: &ThemeColorOverrides) {
        for label in ColorLabel::ALL {
            if let Some(s) = overrides.get(label)
                && let Some(c) = parse_color(s) {
                    self.set_color(label, c);
                }
        }
    }

    pub fn color_at(&self, label: ColorLabel) -> Color {
        match label {
            ColorLabel::UserMessage => self.user_message,
            ColorLabel::AssistantMessageFg => self.assistant_message_fg,
            ColorLabel::AssistantMessageBg => self.assistant_message_bg,
            ColorLabel::SystemMessage => self.system_message,
            ColorLabel::Dialogue => self.dialogue,
            ColorLabel::BorderFocused => self.border_focused,
            ColorLabel::BorderUnfocused => self.border_unfocused,
            ColorLabel::StatusBarFg => self.status_bar_fg,
            ColorLabel::StatusBarBg => self.status_bar_bg,
            ColorLabel::StatusErrorFg => self.status_error_fg,
            ColorLabel::StatusErrorBg => self.status_error_bg,
            ColorLabel::StatusInfoFg => self.status_info_fg,
            ColorLabel::StatusInfoBg => self.status_info_bg,
            ColorLabel::StatusWarningFg => self.status_warning_fg,
            ColorLabel::StatusWarningBg => self.status_warning_bg,
            ColorLabel::NavCursorFg => self.nav_cursor_fg,
            ColorLabel::NavCursorBg => self.nav_cursor_bg,
            ColorLabel::HoverBg => self.hover_bg,
            ColorLabel::SidebarHighlightFg => self.sidebar_highlight_fg,
            ColorLabel::SidebarHighlightBg => self.sidebar_highlight_bg,
            ColorLabel::Dimmed => self.dimmed,
            ColorLabel::CommandPickerFg => self.command_picker_fg,
            ColorLabel::CommandPickerBg => self.command_picker_bg,
            ColorLabel::StreamingIndicator => self.streaming_indicator,
            ColorLabel::ApiUnavailable => self.api_unavailable,
            ColorLabel::SummaryIndicator => self.summary_indicator,
        }
    }

    fn set_color(&mut self, label: ColorLabel, color: Color) {
        let slot = match label {
            ColorLabel::UserMessage => &mut self.user_message,
            ColorLabel::AssistantMessageFg => &mut self.assistant_message_fg,
            ColorLabel::AssistantMessageBg => &mut self.assistant_message_bg,
            ColorLabel::SystemMessage => &mut self.system_message,
            ColorLabel::Dialogue => &mut self.dialogue,
            ColorLabel::BorderFocused => &mut self.border_focused,
            ColorLabel::BorderUnfocused => &mut self.border_unfocused,
            ColorLabel::StatusBarFg => &mut self.status_bar_fg,
            ColorLabel::StatusBarBg => &mut self.status_bar_bg,
            ColorLabel::StatusErrorFg => &mut self.status_error_fg,
            ColorLabel::StatusErrorBg => &mut self.status_error_bg,
            ColorLabel::StatusInfoFg => &mut self.status_info_fg,
            ColorLabel::StatusInfoBg => &mut self.status_info_bg,
            ColorLabel::StatusWarningFg => &mut self.status_warning_fg,
            ColorLabel::StatusWarningBg => &mut self.status_warning_bg,
            ColorLabel::NavCursorFg => &mut self.nav_cursor_fg,
            ColorLabel::NavCursorBg => &mut self.nav_cursor_bg,
            ColorLabel::HoverBg => &mut self.hover_bg,
            ColorLabel::SidebarHighlightFg => &mut self.sidebar_highlight_fg,
            ColorLabel::SidebarHighlightBg => &mut self.sidebar_highlight_bg,
            ColorLabel::Dimmed => &mut self.dimmed,
            ColorLabel::CommandPickerFg => &mut self.command_picker_fg,
            ColorLabel::CommandPickerBg => &mut self.command_picker_bg,
            ColorLabel::StreamingIndicator => &mut self.streaming_indicator,
            ColorLabel::ApiUnavailable => &mut self.api_unavailable,
            ColorLabel::SummaryIndicator => &mut self.summary_indicator,
        };
        *slot = color;
    }

    pub fn available_themes() -> &'static [&'static str] {
        &["dark", "light"]
    }
}

pub fn resolve_theme(config: &Config) -> Theme {
    let name = config.theme.as_deref().unwrap_or("dark");
    let mut theme = Theme::from_name(name).unwrap_or_else(Theme::dark);
    if let Some(ref overrides) = config.theme_colors {
        theme.apply_overrides(overrides);
    }
    theme
}

pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.is_ascii() && hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::Rgb(r, g, b));
        }
        return None;
    }

    if let Some(rest) = s.strip_prefix("indexed(").and_then(|r| r.strip_suffix(')')) {
        let n: u8 = rest.trim().parse().ok()?;
        return Some(Color::Indexed(n));
    }

    match s.to_lowercase().as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "dark_gray" | "dark_grey" | "darkgray" | "darkgrey" => Some(Color::DarkGray),
        "light_red" | "lightred" => Some(Color::LightRed),
        "light_green" | "lightgreen" => Some(Color::LightGreen),
        "light_yellow" | "lightyellow" => Some(Color::LightYellow),
        "light_blue" | "lightblue" => Some(Color::LightBlue),
        "light_magenta" | "lightmagenta" => Some(Color::LightMagenta),
        "light_cyan" | "lightcyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libllm::config::{Config, ThemeColorOverrides};

    #[test]
    fn parse_color_named() {
        assert_eq!(parse_color("red"), Some(Color::Red));
        assert_eq!(parse_color("green"), Some(Color::Green));
        assert_eq!(parse_color("dark_gray"), Some(Color::DarkGray));
        assert_eq!(parse_color("darkgray"), Some(Color::DarkGray));
        assert_eq!(parse_color("light_blue"), Some(Color::LightBlue));
        assert_eq!(parse_color("lightblue"), Some(Color::LightBlue));
        assert_eq!(parse_color("white"), Some(Color::White));
        assert_eq!(parse_color("black"), Some(Color::Black));
    }

    #[test]
    fn parse_color_hex() {
        assert_eq!(parse_color("#ff0000"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(parse_color("#00ff00"), Some(Color::Rgb(0, 255, 0)));
        assert_eq!(parse_color("#1a2b3c"), Some(Color::Rgb(26, 43, 60)));
    }

    #[test]
    fn parse_color_indexed() {
        assert_eq!(parse_color("indexed(236)"), Some(Color::Indexed(236)));
        assert_eq!(parse_color("indexed(0)"), Some(Color::Indexed(0)));
    }

    #[test]
    fn parse_color_invalid() {
        assert_eq!(parse_color("notacolor"), None);
        assert_eq!(parse_color("#xyz"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("indexed(abc)"), None);
        assert_eq!(parse_color(""), None);
    }

    #[test]
    fn parse_color_non_ascii_hex_returns_none() {
        // "àáâ" encodes as 6 UTF-8 bytes but contains 3 non-ASCII chars.
        // Byte-index slicing would panic; the ASCII guard must reject it.
        assert_eq!(parse_color("#àáâ"), None);
    }

    #[test]
    fn parse_color_case_insensitive() {
        assert_eq!(parse_color("RED"), Some(Color::Red));
        assert_eq!(parse_color("Dark_Gray"), Some(Color::DarkGray));
        assert_eq!(parse_color("LightBlue"), Some(Color::LightBlue));
    }

    #[test]
    fn parse_color_with_whitespace() {
        assert_eq!(parse_color("  red  "), Some(Color::Red));
        assert_eq!(parse_color(" #ff0000 "), Some(Color::Rgb(255, 0, 0)));
    }

    #[test]
    fn from_name_dark() {
        assert!(Theme::from_name("dark").is_some());
    }

    #[test]
    fn from_name_light() {
        assert!(Theme::from_name("light").is_some());
    }

    #[test]
    fn from_name_unknown() {
        assert!(Theme::from_name("solarized").is_none());
        assert!(Theme::from_name("").is_none());
    }

    #[test]
    fn resolve_default() {
        let config = Config::default();
        let t = resolve_theme(&config);
        assert_eq!(t.user_message, Color::Green);
    }

    #[test]
    fn resolve_light() {
        let config = Config {
            theme: Some("light".to_owned()),
            ..Config::default()
        };
        let t = resolve_theme(&config);
        assert_eq!(t.user_message, Color::Blue);
    }

    #[test]
    fn resolve_with_overrides() {
        let config = Config {
            theme_colors: Some(ThemeColorOverrides {
                user_message: Some("red".to_owned()),
                ..Default::default()
            }),
            ..Config::default()
        };
        let t = resolve_theme(&config);
        assert_eq!(t.user_message, Color::Red);
    }

    #[test]
    fn resolve_invalid_override_ignored() {
        let config = Config {
            theme_colors: Some(ThemeColorOverrides {
                user_message: Some("notacolor".to_owned()),
                ..Default::default()
            }),
            ..Config::default()
        };
        let t = resolve_theme(&config);
        assert_eq!(t.user_message, Color::Green);
    }

    #[test]
    fn available_themes_not_empty() {
        let themes = Theme::available_themes();
        assert!(themes.contains(&"dark"));
        assert!(themes.contains(&"light"));
    }
}
