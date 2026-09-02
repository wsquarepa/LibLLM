//! Application state management: autosave scheduling, status messages, and notification timers.

use anyhow::Result;
use tui_textarea::TextArea;

use super::App;
use super::dialogs;
use super::types::{
    AUTOSAVE_DEBOUNCE, AUTOSAVE_RETRY_DELAY, NOTIFICATION_SLIDE_DURATION, STATUS_DURATION,
    SaveTrigger, StatusLevel, StatusMessage,
};

/// Clears `flash` once its highlight window has elapsed.
///
/// Returns whether `flash` was set on entry, which is whether a redraw is needed.
fn tick_flash(flash: &mut Option<std::time::Instant>) -> bool {
    let Some(started) = *flash else {
        return false;
    };
    if !dialogs::is_flash_active(Some(started)) {
        *flash = None;
    }
    true
}

impl App<'_> {
    pub(super) fn can_persist_session(&self) -> bool {
        matches!(
            self.save_mode,
            libllm_core::session::SaveMode::Database { .. }
        ) && self.db.is_some()
            && self.session_has_user_message()
    }

    fn session_has_user_message(&self) -> bool {
        self.session
            .tree
            .nodes()
            .iter()
            .any(|node| node.message.role == libllm_core::session::Role::User)
    }

    pub(super) fn tick_reject_flashes(&mut self) -> bool {
        let mut needs_redraw = tick_flash(&mut self.input_reject_flash);
        for flash in [
            self.config_dialog.as_mut().map(|d| &mut d.reject_flash),
            self.theme_ui.dialog.as_mut().map(|d| &mut d.reject_flash),
            self.persona.editor.as_mut().map(|d| &mut d.reject_flash),
            self.system_prompt
                .editor
                .as_mut()
                .map(|d| &mut d.reject_flash),
            self.character.editor.as_mut().map(|d| &mut d.reject_flash),
            self.worldbook
                .entry_editor
                .as_mut()
                .map(|d| &mut d.reject_flash),
        ]
        .into_iter()
        .flatten()
        {
            needs_redraw |= tick_flash(flash);
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
        self.display_regex_cache.clear();
    }

    pub(super) fn prefill_display_regex_cache(&mut self) {
        if self.compiled_regex.is_empty() {
            return;
        }
        let ids: Vec<libllm_core::session::NodeId> =
            self.session.tree.current_branch_ids().to_vec();
        for id in ids {
            if self.display_regex_cache.contains_key(&id) {
                continue;
            }
            let Some(node) = self.session.tree.node(id) else {
                continue;
            };
            let role = node.message.role;
            let transformed = libllm_core::regex_rules::apply(
                &self.compiled_regex,
                libllm_core::regex_rules::Scope::Display,
                role,
                &node.message.content,
            )
            .into_owned();
            self.display_regex_cache.insert(id, transformed);
        }
    }

    pub(super) fn display_content_for(
        &self,
        node_id: libllm_core::session::NodeId,
    ) -> Option<&str> {
        if self.compiled_regex.is_empty() {
            return None;
        }
        self.display_regex_cache.get(&node_id).map(String::as_str)
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
        self.autosave.dirty = true;
        self.autosave.trigger = Some(trigger);
        if self.can_persist_session() {
            let deadline = if immediate {
                std::time::Instant::now()
            } else {
                std::time::Instant::now() + AUTOSAVE_DEBOUNCE
            };
            self.autosave.deadline = Some(deadline);
        }
        if self.autosave.debug.dirty_since.is_none() {
            self.autosave.debug.dirty_since = Some(std::time::Instant::now());
        }
        tracing::debug!(
            phase = "schedule",
            trigger = trigger.as_str(),
            persistable = self.can_persist_session(),
            session_dirty = self.autosave.dirty,
            "autosave",
        );
    }

    pub(super) fn discard_pending_session_save(&mut self) {
        self.autosave.dirty = false;
        self.autosave.deadline = None;
        self.autosave.trigger = None;
        self.autosave.debug.dirty_since = None;
    }

    pub(super) fn flush_session_save(&mut self, trigger: SaveTrigger) -> Result<()> {
        if !self.autosave.dirty || !self.can_persist_session() {
            tracing::debug!(
                phase = "flush",
                trigger = trigger.as_str(),
                result = "skipped",
                session_dirty = self.autosave.dirty,
                persistable = self.can_persist_session(),
                "autosave",
            );
            return Ok(());
        }

        let dirty_elapsed_ms = self
            .autosave
            .debug
            .dirty_since
            .map(|started| started.elapsed().as_secs_f64() * 1000.0);

        let session_id = self.save_mode.id().map(str::to_owned);
        let start = std::time::Instant::now();
        let result = libllm_storage::db::save_session_for_mode(
            &self.save_mode,
            self.session,
            self.db.as_mut(),
        );
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

        match result {
            Ok(()) => {
                self.autosave.debug.save_count += 1;
                tracing::debug!(
                    phase = "flush",
                    trigger = trigger.as_str(),
                    result = "ok",
                    elapsed_ms = elapsed_ms,
                    session_id = session_id.as_deref(),
                    dirty_elapsed_ms = ?dirty_elapsed_ms,
                    save_count = self.autosave.debug.save_count,
                    "autosave",
                );
                self.discard_pending_session_save();
                Ok(())
            }
            Err(err) => {
                self.autosave.deadline = Some(std::time::Instant::now() + AUTOSAVE_RETRY_DELAY);
                self.autosave.trigger = Some(SaveTrigger::Retry);
                self.autosave.debug.retry_count += 1;
                tracing::warn!(
                    phase = "flush",
                    trigger = trigger.as_str(),
                    result = "error",
                    elapsed_ms = elapsed_ms,
                    retry_delay_ms = AUTOSAVE_RETRY_DELAY.as_millis(),
                    error = %err,
                    session_id = session_id.as_deref(),
                    dirty_elapsed_ms = ?dirty_elapsed_ms,
                    retry_count = self.autosave.debug.retry_count,
                    "autosave",
                );
                Err(err.into())
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
