use anyhow::Result;

use super::dialogs;
use super::types::{SaveTrigger, StatusLevel, StatusMessage, AUTOSAVE_DEBOUNCE, AUTOSAVE_RETRY_DELAY, NOTIFICATION_SLIDE_DURATION, STATUS_DURATION};
use super::App;

impl App<'_> {
    pub(super) fn can_persist_session(&self) -> bool {
        matches!(self.save_mode, libllm::session::SaveMode::Database { .. }) && self.db.is_some()
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
        for dialog in [
            &mut self.config_dialog,
            &mut self.persona_editor,
            &mut self.system_prompt_editor,
            &mut self.character_editor,
            &mut self.worldbook_entry_editor,
        ] {
            if let Some(d) = dialog.as_mut() {
                if let Some(t) = d.reject_flash {
                    if dialogs::is_flash_active(Some(t)) {
                        needs_redraw = true;
                    } else {
                        d.reject_flash = None;
                        needs_redraw = true;
                    }
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

    pub(super) fn invalidate_chat_cache(&mut self) {
        self.chat_content_cache = None;
        self.cached_token_count = None;
    }

    pub(super) fn invalidate_worldbook_cache(&mut self) {
        self.worldbook_cache = None;
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
        libllm::debug_log::log_kv(
            "autosave",
            &[
                libllm::debug_log::field("phase", "schedule"),
                libllm::debug_log::field("trigger", trigger.as_str()),
                libllm::debug_log::field("persistable", self.can_persist_session()),
                libllm::debug_log::field("session_dirty", self.session_dirty),
            ],
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
            libllm::debug_log::log_kv(
                "autosave",
                &[
                    libllm::debug_log::field("phase", "flush"),
                    libllm::debug_log::field("trigger", trigger.as_str()),
                    libllm::debug_log::field("result", "skipped"),
                    libllm::debug_log::field("session_dirty", self.session_dirty),
                    libllm::debug_log::field("persistable", self.can_persist_session()),
                ],
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

        match result {
            Ok(()) => {
                self.autosave_debug.save_count += 1;
                let mut fields = vec![
                    libllm::debug_log::field("phase", "flush"),
                    libllm::debug_log::field("trigger", trigger.as_str()),
                    libllm::debug_log::field("result", "ok"),
                    libllm::debug_log::field("elapsed_ms", format!("{elapsed_ms:.3}")),
                ];
                if let Some(ref sid) = session_id {
                    fields.push(libllm::debug_log::field("session_id", sid));
                }
                if let Some(dirty_elapsed_ms) = dirty_elapsed_ms {
                    fields.push(libllm::debug_log::field(
                        "dirty_elapsed_ms",
                        format!("{dirty_elapsed_ms:.3}"),
                    ));
                }
                fields.push(libllm::debug_log::field(
                    "save_count",
                    self.autosave_debug.save_count,
                ));
                libllm::debug_log::log_kv("autosave", &fields);
                self.discard_pending_session_save();
                Ok(())
            }
            Err(err) => {
                self.pending_save_deadline = Some(std::time::Instant::now() + AUTOSAVE_RETRY_DELAY);
                self.pending_save_trigger = Some(SaveTrigger::Retry);
                self.autosave_debug.retry_count += 1;
                let mut fields = vec![
                    libllm::debug_log::field("phase", "flush"),
                    libllm::debug_log::field("trigger", trigger.as_str()),
                    libllm::debug_log::field("result", "error"),
                    libllm::debug_log::field("elapsed_ms", format!("{elapsed_ms:.3}")),
                    libllm::debug_log::field("retry_delay_ms", AUTOSAVE_RETRY_DELAY.as_millis()),
                    libllm::debug_log::field("error", &err),
                ];
                if let Some(ref sid) = session_id {
                    fields.push(libllm::debug_log::field("session_id", sid));
                }
                if let Some(dirty_elapsed_ms) = dirty_elapsed_ms {
                    fields.push(libllm::debug_log::field(
                        "dirty_elapsed_ms",
                        format!("{dirty_elapsed_ms:.3}"),
                    ));
                }
                fields.push(libllm::debug_log::field(
                    "retry_count",
                    self.autosave_debug.retry_count,
                ));
                libllm::debug_log::log_kv("autosave", &fields);
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
