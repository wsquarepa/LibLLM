use ratatui::style::Color;

use crate::config::{Config, ThemeColorOverrides};

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
        macro_rules! apply {
            ($field:ident) => {
                if let Some(ref s) = overrides.$field {
                    if let Some(c) = parse_color(s) {
                        self.$field = c;
                    }
                }
            };
        }
        apply!(user_message);
        apply!(assistant_message_fg);
        apply!(assistant_message_bg);
        apply!(system_message);
        apply!(border_focused);
        apply!(border_unfocused);
        apply!(status_bar_fg);
        apply!(status_bar_bg);
        apply!(status_error_fg);
        apply!(status_error_bg);
        apply!(status_info_fg);
        apply!(status_info_bg);
        apply!(status_warning_fg);
        apply!(status_warning_bg);
        apply!(dialogue);
        apply!(nav_cursor_fg);
        apply!(nav_cursor_bg);
        apply!(hover_bg);
        apply!(dimmed);
        apply!(sidebar_highlight_fg);
        apply!(sidebar_highlight_bg);
        apply!(command_picker_fg);
        apply!(command_picker_bg);
        apply!(streaming_indicator);
        apply!(api_unavailable);
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
        if hex.len() == 6 {
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
