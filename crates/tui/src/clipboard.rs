//! System clipboard read/write with platform-specific handling.

use arboard::Clipboard;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_textarea::TextArea;

#[cfg(target_os = "linux")]
use arboard::SetExtLinux;

fn set_clipboard(text: &str) -> Result<(), String> {
    let text = text.to_owned();

    #[cfg(target_os = "linux")]
    {
        std::thread::spawn(move || {
            let mut cb = match Clipboard::new() {
                Ok(cb) => cb,
                Err(_) => return,
            };
            let _ = cb.set().wait().text(text);
        });
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        Clipboard::new()
            .and_then(|mut cb| cb.set_text(text))
            .map_err(|e| format!("Clipboard: {e}"))
    }
}

fn get_clipboard() -> Result<String, String> {
    Clipboard::new()
        .and_then(|mut cb| cb.get_text())
        .map_err(|e| format!("Clipboard: {e}"))
}

/// Intercept Ctrl+C/V/X for system clipboard on a TextArea.
///
/// Returns `(consumed, optional_warning)`. When `consumed` is false the caller
/// should forward the key to `textarea.input()` or handle it otherwise.
pub fn handle_clipboard_key(key: &KeyEvent, textarea: &mut TextArea<'_>) -> (bool, Option<String>) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match (ctrl, key.code) {
        (true, KeyCode::Char('c')) => {
            if textarea.selection_range().is_some() {
                textarea.copy();
                let warning = set_clipboard(&textarea.yank_text()).err();
                (true, warning)
            } else {
                (false, None)
            }
        }
        (true, KeyCode::Char('x')) => {
            if textarea.selection_range().is_some() {
                textarea.cut();
                let warning = set_clipboard(&textarea.yank_text()).err();
                (true, warning)
            } else {
                (false, None)
            }
        }
        (true, KeyCode::Char('v')) => match get_clipboard() {
            Ok(text) if !text.is_empty() => {
                textarea.insert_str(&text);
                (true, None)
            }
            Ok(_) => (true, None),
            Err(msg) => (true, Some(msg)),
        },
        _ => (false, None),
    }
}
