//! Per-keystroke and on-commit field validation for text, numeric ranges, length limits, and color values.

use crate::tui::theme::parse_color;

#[derive(Clone, Copy)]
pub enum FieldValidation {
    Float { min: f64, max: f64 },
    Int { min: i64, max: i64 },
    MaxLen(usize),
    Color,
}

impl FieldValidation {
    fn max_digits(max_abs: u64) -> usize {
        if max_abs == 0 {
            1
        } else {
            (max_abs as f64).log10().floor() as usize + 1
        }
    }

    pub(super) fn accepts_char(&self, current: &str, c: char) -> bool {
        match self {
            Self::Float { min, max } => {
                if c == '-' {
                    return *min < 0.0 && current.is_empty();
                }
                if c == '.' {
                    return !current.contains('.');
                }
                if !c.is_ascii_digit() {
                    return false;
                }
                let digits_only = current.trim_start_matches('-');
                if let Some(dot_pos) = digits_only.find('.') {
                    digits_only.len() - dot_pos <= 2
                } else {
                    let max_whole = Self::max_digits(max.abs() as u64);
                    digits_only.len() < max_whole
                }
            }
            Self::Int { min, max } => {
                if c == '-' {
                    *min < 0 && current.is_empty()
                } else if c.is_ascii_digit() {
                    let digits_only = current.trim_start_matches('-');
                    let max_abs = (*min).unsigned_abs().max((*max).unsigned_abs());
                    digits_only.len() < Self::max_digits(max_abs)
                } else {
                    false
                }
            }
            Self::MaxLen(max) => current.chars().count() < *max,
            // Color input spans multiple formats; validity can only be determined on commit.
            Self::Color => true,
        }
    }

    pub(super) fn validate(&self, value: &str) -> bool {
        match self {
            Self::Color => value.is_empty() || parse_color(value).is_some(),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_validates_empty_as_inherit() {
        let v = FieldValidation::Color;
        assert!(v.validate(""));
    }

    #[test]
    fn color_validates_hex() {
        let v = FieldValidation::Color;
        assert!(v.validate("#ff0000"));
        assert!(v.validate("#1a2b3c"));
    }

    #[test]
    fn color_validates_named() {
        let v = FieldValidation::Color;
        assert!(v.validate("red"));
        assert!(v.validate("dark_gray"));
    }

    #[test]
    fn color_validates_indexed() {
        let v = FieldValidation::Color;
        assert!(v.validate("indexed(236)"));
    }

    #[test]
    fn color_rejects_nonsense() {
        let v = FieldValidation::Color;
        assert!(!v.validate("not_a_color"));
        assert!(!v.validate("#xyz"));
        assert!(!v.validate("#12345"));
    }

    #[test]
    fn color_accepts_any_char_during_typing() {
        let v = FieldValidation::Color;
        assert!(v.accepts_char("", '#'));
        assert!(v.accepts_char("#ff", 'a'));
        assert!(v.accepts_char("indexed(", '2'));
    }
}
