//! Application state management: autosave scheduling, status messages, and notification timers.

use anyhow::Result;
use tui_textarea::TextArea;

use super::App;
use super::dialogs;
use super::types::{
    AUTOSAVE_DEBOUNCE, AUTOSAVE_RETRY_DELAY, NOTIFICATION_SLIDE_DURATION, STATUS_DURATION,
    SaveTrigger, StatusLevel, StatusMessage,
};

impl App<'_> {
    pub(super) fn can_persist_session(&self) -> bool {
        matches!(self.save_mode, libllm::session::SaveMode::Database { .. })
            && self.db.is_some()
            && self.session_has_user_message()
    }

    fn session_has_user_message(&self) -> bool {
        self.session
            .tree
            .nodes()
            .iter()
            .any(|node| node.message.role == libllm::session::Role::User)
    }

    pub(super) fn tick_reject_flashes(&mut self) -> bool {
        let mut needs_redraw = false;
        if let Some(t) = self.input_reject_flash {
            if dialogs::is_flash_active(Some(t)) {
                needs_redraw = true;
            } else {
                self.input_reject_flash = None;
                needs_redraw = true;
            }
        }
        if let Some(d) = self.config_dialog.as_mut()
            && let Some(t) = d.reject_flash
        {
            if dialogs::is_flash_active(Some(t)) {
                needs_redraw = true;
            } else {
                d.reject_flash = None;
                needs_redraw = true;
            }
        }
        if let Some(d) = self.theme_dialog.as_mut()
            && let Some(t) = d.reject_flash
        {
            if dialogs::is_flash_active(Some(t)) {
                needs_redraw = true;
            } else {
                d.reject_flash = None;
                needs_redraw = true;
            }
        }
        for dialog in [
            &mut self.persona_editor,
            &mut self.system_prompt_editor,
            &mut self.character_editor,
            &mut self.worldbook_entry_editor,
        ] {
            if let Some(d) = dialog.as_mut()
                && let Some(t) = d.reject_flash
            {
                if dialogs::is_flash_active(Some(t)) {
                    needs_redraw = true;
                } else {
                    d.reject_flash = None;
                    needs_redraw = true;
                }
            }
        }
        needs_redraw
    }

    const MAX_STATUS_LENGTH: usize = 64;

    pub(super) fn set_status(&mut self, text: String, level: StatusLevel) {
        let now = std::time::Instant::now();
        let created = if self.status_message.is_some() {
            now - NOTIFICATION_SLIDE_DURATION
        } else {
            now
        };
        let truncated = if text.len() > Self::MAX_STATUS_LENGTH {
            let end = text.floor_char_boundary(Self::MAX_STATUS_LENGTH - 3);
            format!("{}...", &text[..end])
        } else {
            text
        };
        self.status_message = Some(StatusMessage {
            text: truncated,
            level,
            created,
            expires: now + STATUS_DURATION,
        });
    }

    pub(super) fn invalidate_chat_render_cache(&mut self) {
        self.chat_content_cache = None;
    }

    pub(super) fn invalidate_prompt_cache(&mut self) {
        self.cached_token_count = None;
    }

    pub(super) fn invalidate_chat_caches(&mut self) {
        self.invalidate_chat_render_cache();
        self.invalidate_prompt_cache();
    }

    /// Clear the textarea only when it still holds `submitted_content` (trimmed).
    /// Used by the send pipeline so that messages originating from the queue
    /// (re-sent after an `Esc` cancel) don't wipe out new text the user has
    /// typed in the meantime.
    pub(super) fn clear_input_textarea_if_holds(&mut self, submitted_content: &str) {
        let current = self.textarea.lines().join("\n");
        if current.trim() == submitted_content.trim() {
            self.textarea = TextArea::default();
            super::dialog_handler::configure_textarea(&mut self.textarea);
            self.command_picker_selected = 0;
        }
    }

    pub(super) fn invalidate_worldbook_cache(&mut self) {
        self.worldbook_cache = None;
        self.invalidate_prompt_cache();
    }

    pub(super) fn mark_session_dirty(&mut self, trigger: SaveTrigger, immediate: bool) {
        self.session_dirty = true;
        self.pending_save_trigger = Some(trigger);
        if self.can_persist_session() {
            let deadline = if immediate {
                std::time::Instant::now()
            } else {
                std::time::Instant::now() + AUTOSAVE_DEBOUNCE
            };
            self.pending_save_deadline = Some(deadline);
        }
        if self.autosave_debug.dirty_since.is_none() {
            self.autosave_debug.dirty_since = Some(std::time::Instant::now());
        }
        tracing::debug!(
            phase = "schedule",
            trigger = trigger.as_str(),
            persistable = self.can_persist_session(),
            session_dirty = self.session_dirty,
            "autosave",
        );
    }

    pub(super) fn discard_pending_session_save(&mut self) {
        self.session_dirty = false;
        self.pending_save_deadline = None;
        self.pending_save_trigger = None;
        self.autosave_debug.dirty_since = None;
    }

    pub(super) fn flush_session_save(&mut self, trigger: SaveTrigger) -> Result<()> {
        if !self.session_dirty || !self.can_persist_session() {
            tracing::debug!(
                phase = "flush",
                trigger = trigger.as_str(),
                result = "skipped",
                session_dirty = self.session_dirty,
                persistable = self.can_persist_session(),
                "autosave",
            );
            return Ok(());
        }

        let dirty_elapsed_ms = self
            .autosave_debug
            .dirty_since
            .map(|started| started.elapsed().as_secs_f64() * 1000.0);

        let session_id = self.save_mode.id().map(str::to_owned);
        let start = std::time::Instant::now();
        let result = self.session.maybe_save(&self.save_mode, self.db.as_mut());
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

        let elapsed_ms_str = format!("{elapsed_ms:.3}");
        let dirty_elapsed_ms_str = dirty_elapsed_ms.map(|ms| format!("{ms:.3}"));
        match result {
            Ok(()) => {
                self.autosave_debug.save_count += 1;
                tracing::debug!(
                    phase = "flush",
                    trigger = trigger.as_str(),
                    result = "ok",
                    elapsed_ms = elapsed_ms_str,
                    session_id = session_id.as_deref(),
                    dirty_elapsed_ms = dirty_elapsed_ms_str.as_deref(),
                    save_count = self.autosave_debug.save_count,
                    "autosave",
                );
                self.discard_pending_session_save();
                Ok(())
            }
            Err(err) => {
                self.pending_save_deadline = Some(std::time::Instant::now() + AUTOSAVE_RETRY_DELAY);
                self.pending_save_trigger = Some(SaveTrigger::Retry);
                self.autosave_debug.retry_count += 1;
                tracing::warn!(
                    phase = "flush",
                    trigger = trigger.as_str(),
                    result = "error",
                    elapsed_ms = elapsed_ms_str,
                    retry_delay_ms = AUTOSAVE_RETRY_DELAY.as_millis(),
                    error = %err,
                    session_id = session_id.as_deref(),
                    dirty_elapsed_ms = dirty_elapsed_ms_str.as_deref(),
                    retry_count = self.autosave_debug.retry_count,
                    "autosave",
                );
                Err(err)
            }
        }
    }

    pub(super) fn flush_session_before_transition(&mut self) -> bool {
        match self.flush_session_save(SaveTrigger::Transition) {
            Ok(()) => true,
            Err(err) => {
                self.set_status(format!("Save error: {err}"), StatusLevel::Error);
                false
            }
        }
    }
}
