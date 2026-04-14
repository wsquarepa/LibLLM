#[derive(Clone, Copy)]
pub enum FieldValidation {
    Float { min: f64, max: f64 },
    Int { min: i64, max: i64 },
    MaxLen(usize),
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
        }
    }
}
