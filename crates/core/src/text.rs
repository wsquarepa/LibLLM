//! Text sanitization helpers shared across crates.

/// Removes terminal control sequences from `input` while preserving the FTS
/// highlight marker bytes U+0001 and U+0002 that downstream renderers depend on.
///
/// Strips:
/// - All C0 controls (U+0000–U+001F) except U+0001 (HIGHLIGHT_OPEN) and U+0002 (HIGHLIGHT_CLOSE)
/// - U+007F DEL
/// - All C1 controls (U+0080–U+009F)
/// - ANSI/VT escape sequences: CSI (ESC [), OSC/DCS/SOS/PM/APC (ESC ] / P / X / ^ / _),
///   and bare ESC + any single character
pub fn strip_terminal_controls(input: &str) -> String {
    enum State {
        Normal,
        Esc,
        CsiSeq,
        OscSeq,
    }

    let mut out = String::with_capacity(input.len());
    let mut state = State::Normal;

    for c in input.chars() {
        match state {
            State::Normal => match c {
                '\u{0001}' | '\u{0002}' => out.push(c),
                '\u{001B}' => state = State::Esc,
                c if (c as u32) <= 0x1F => {}
                '\u{007F}' => {}
                c if (c as u32) >= 0x80 && (c as u32) <= 0x9F => {}
                _ => out.push(c),
            },
            State::Esc => match c {
                '[' => state = State::CsiSeq,
                ']' | 'P' | 'X' | '^' | '_' => state = State::OscSeq,
                _ => state = State::Normal,
            },
            State::CsiSeq => {
                // CSI sequence ends on any byte in the range 0x40–0x7E
                if ('\u{0040}'..='\u{007E}').contains(&c) {
                    state = State::Normal;
                }
            }
            State::OscSeq => match c {
                // BEL terminates OSC/DCS/SOS/PM/APC
                '\u{0007}' => state = State::Normal,
                // ST = ESC backslash; the ESC here transitions us back through Esc state
                '\u{001B}' => state = State::Esc,
                _ => {}
            },
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::strip_terminal_controls;

    #[test]
    fn strip_controls_passes_plain_text() {
        assert_eq!(strip_terminal_controls("hello world"), "hello world");
    }

    #[test]
    fn strip_controls_removes_csi_sequence() {
        assert_eq!(
            strip_terminal_controls("before\x1b[31mRED\x1b[0mafter"),
            "beforeREDafter",
        );
    }

    #[test]
    fn strip_controls_removes_osc52() {
        assert_eq!(
            strip_terminal_controls("pre\x1b]52;c;U0VDUkVU\x07post"),
            "prepost",
        );
    }

    #[test]
    fn strip_controls_removes_osc_with_st_terminator() {
        assert_eq!(
            strip_terminal_controls("pre\x1b]0;title\x1b\\post"),
            "prepost",
        );
    }

    #[test]
    fn strip_controls_removes_bare_esc_plus_single_byte() {
        assert_eq!(strip_terminal_controls("\x1bcX"), "X");
    }

    #[test]
    fn strip_controls_removes_csi_erase_display() {
        assert_eq!(strip_terminal_controls("\x1b[2J"), "");
    }

    #[test]
    fn strip_controls_preserves_fts_markers() {
        let input = "\u{1}word\u{2}";
        assert_eq!(strip_terminal_controls(input), "\u{1}word\u{2}");
    }

    #[test]
    fn strip_controls_removes_c0_except_markers() {
        // \t = U+0009, \0 = U+0000, \x03 = ETX — all C0 except U+0001 and U+0002
        let input = "\t\0\x03\u{1}hi\u{2}";
        assert_eq!(strip_terminal_controls(input), "\u{1}hi\u{2}");
    }

    #[test]
    fn strip_controls_removes_c1_range() {
        // U+0080 through U+009F are C1 controls
        let input = "a\u{0080}b\u{009f}c".to_string();
        assert_eq!(strip_terminal_controls(&input), "abc");
    }

    #[test]
    fn strip_controls_removes_del() {
        assert_eq!(strip_terminal_controls("a\x7fb"), "ab");
    }

    #[test]
    fn strip_controls_bare_esc_at_end_of_string_does_not_panic() {
        let input = "text\x1b";
        assert_eq!(strip_terminal_controls(input), "text");
    }
}
