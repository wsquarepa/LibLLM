//! Background event processing for passkey derivation and model name resolution.

use std::path::Path;

use libllm::db::Database;
use libllm::session::{self, SaveMode};

use crate::tui::business;
use crate::tui::types::{BackgroundEvent, Focus, StatusLevel};

use super::App;

fn prepare_backup_rekey(
    data_dir: &Path,
    old_passkey: &str,
    new_passkey: &str,
) -> anyhow::Result<()> {
    let old_kek = backup::crypto::resolve_backup_key(data_dir, Some(old_passkey))?;
    let new_kek = backup::crypto::resolve_backup_key(data_dir, Some(new_passkey))?;
    match (old_kek, new_kek) {
        (Some(ok), Some(nk)) => backup::rekey::prepare_rekey(data_dir, &ok, &nk),
        _ => Ok(()),
    }
}

fn post_passkey_focus(app: &App) -> Focus {
    if app.model_name.is_none() {
        Focus::LoadingDialog
    } else if !app.api_available {
        Focus::ApiErrorDialog
    } else {
        Focus::Input
    }
}

pub(in crate::tui) fn handle_background_event(event: BackgroundEvent, app: &mut App) {
    match event {
        BackgroundEvent::KeyDerived(key, db_path) => {
            if let Some(unlock_dbg) = app.unlock_debug.take() {
                let elapsed_ms = format!(
                    "{:.3}",
                    unlock_dbg.started_at.elapsed().as_secs_f64() * 1000.0
                );
                tracing::info!(
                    phase = "ui_complete",
                    kind = unlock_dbg.kind,
                    result = "ok",
                    elapsed_ms = elapsed_ms.as_str(),
                    "unlock.phase"
                );
            }
            match Database::open(&db_path, Some(&key)) {
                Ok(db) => {
                    if let Err(e) = db.ensure_builtin_prompts() {
                        app.set_status(format!("Warning: {e}"), StatusLevel::Warning);
                    }
                    libllm::preset::ensure_default_presets();
                    let id = match &app.save_mode {
                        SaveMode::PendingPasskey { id } => id.clone(),
                        _ => session::generate_session_id(),
                    };
                    app.db = Some(db);
                    app.save_mode = SaveMode::Database { id };
                    business::load_active_persona(app);
                    app.invalidate_worldbook_cache();
                    app.invalidate_chat_render_cache();
                    match business::build_file_summarizer(
                        &db_path,
                        Some(&key),
                        &app.config,
                        &app.cli_overrides,
                        app.file_summary_ready_tx.clone(),
                    ) {
                        Ok(fs) => app.file_summarizer = Some(fs),
                        Err(err) => {
                            tracing::error!(
                                error = %err,
                                "tui.file_summarizer.construct.late_failed"
                            );
                            app.set_status(
                                format!("File summaries disabled: {err}"),
                                StatusLevel::Warning,
                            );
                        }
                    }
                    app.passkey_deriving = false;
                    app.focus = post_passkey_focus(app);
                    business::refresh_sidebar(app);
                }
                Err(err) => {
                    app.passkey_deriving = false;
                    app.passkey_error = format!("Failed to open database: {err}");
                }
            }
        }
        BackgroundEvent::KeyDeriveFailed(err) => {
            if let Some(unlock_dbg) = app.unlock_debug.take() {
                let elapsed_ms = format!(
                    "{:.3}",
                    unlock_dbg.started_at.elapsed().as_secs_f64() * 1000.0
                );
                tracing::warn!(phase = "ui_complete", kind = unlock_dbg.kind, result = "error", elapsed_ms = elapsed_ms.as_str(), error = %err, "unlock.phase");
            }
            app.passkey_deriving = false;
            app.passkey_error = format!("Failed: {err}");
            app.resolved_passkey = None;
        }
        BackgroundEvent::PasskeySet(new_key) => {
            if let Some(unlock_dbg) = app.unlock_debug.take() {
                let elapsed_ms = format!(
                    "{:.3}",
                    unlock_dbg.started_at.elapsed().as_secs_f64() * 1000.0
                );
                tracing::info!(
                    phase = "ui_complete",
                    kind = unlock_dbg.kind,
                    result = "ok",
                    elapsed_ms = elapsed_ms.as_str(),
                    "unlock.phase"
                );
            }
            app.set_passkey_deriving = false;
            app.invalidate_worldbook_cache();
            app.invalidate_chat_render_cache();
            if app.set_passkey_is_initial {
                let db_path = libllm::config::data_dir().join("data.db");
                match Database::open(&db_path, Some(&new_key)) {
                    Ok(db) => {
                        if let Err(e) = db.ensure_builtin_prompts() {
                            app.set_status(format!("Warning: {e}"), StatusLevel::Warning);
                        }
                        libllm::preset::ensure_default_presets();
                        let id = match &app.save_mode {
                            SaveMode::PendingPasskey { id } => id.clone(),
                            _ => session::generate_session_id(),
                        };
                        app.db = Some(db);
                        app.save_mode = SaveMode::Database { id };
                        business::load_active_persona(app);
                        if let Err(e) = libllm::config::save(&app.config) {
                            app.set_status(
                                format!("Failed to write default config: {e}"),
                                StatusLevel::Warning,
                            );
                        }
                        match business::build_file_summarizer(
                            &db_path,
                            Some(&new_key),
                            &app.config,
                            &app.cli_overrides,
                            app.file_summary_ready_tx.clone(),
                        ) {
                            Ok(fs) => app.file_summarizer = Some(fs),
                            Err(err) => {
                                tracing::error!(
                                    error = %err,
                                    "tui.file_summarizer.construct.late_failed"
                                );
                                app.set_status(
                                    format!("File summaries disabled: {err}"),
                                    StatusLevel::Warning,
                                );
                            }
                        }
                        app.focus = post_passkey_focus(app);
                        business::refresh_sidebar(app);
                    }
                    Err(err) => {
                        app.set_status(
                            format!("Failed to create database: {err}"),
                            StatusLevel::Error,
                        );
                    }
                }
            } else {
                if let Some(ref db) = app.db {
                    let data_dir = libllm::config::data_dir();
                    let old_passkey = app.resolved_passkey.clone();
                    let new_passkey = app.pending_new_passkey.take();

                    let rekey_result = match (old_passkey.as_deref(), new_passkey.as_deref()) {
                        (Some(old_pw), Some(new_pw)) => {
                            prepare_backup_rekey(&data_dir, old_pw, new_pw)
                        }
                        _ => {
                            tracing::warn!(
                                "passkey change: missing old or new passkey string; skipping backup rewrap"
                            );
                            Ok(())
                        }
                    };

                    if let Err(err) = rekey_result {
                        app.set_status(
                            format!("Failed to rewrap backups: {err}"),
                            StatusLevel::Error,
                        );
                        app.focus = Focus::Input;
                        return;
                    }

                    match db.rekey(&new_key) {
                        Ok(()) => {
                            app.resolved_passkey = new_passkey;
                            if let Err(err) = backup::rekey::finalize_rekey(&data_dir) {
                                app.set_status(
                                    format!("Passkey changed, but failed to clean up backup journal: {err}"),
                                    StatusLevel::Warning,
                                );
                            }
                            app.passkey_changed = true;
                            app.should_quit = true;
                        }
                        Err(err) => {
                            if let Err(rb_err) = backup::rekey::rollback_rekey(&data_dir) {
                                app.set_status(
                                    format!(
                                        "Failed to change passkey: {err}; additionally, backup rollback failed: {rb_err}"
                                    ),
                                    StatusLevel::Error,
                                );
                            } else {
                                app.set_status(
                                    format!("Failed to change passkey: {err}"),
                                    StatusLevel::Error,
                                );
                            }
                        }
                    }
                } else {
                    app.set_status(
                        "No database available for rekey.".to_owned(),
                        StatusLevel::Error,
                    );
                }
                app.focus = Focus::Input;
            }
        }
        BackgroundEvent::PasskeySetFailed(err) => {
            if let Some(unlock_dbg) = app.unlock_debug.take() {
                let elapsed_ms = format!(
                    "{:.3}",
                    unlock_dbg.started_at.elapsed().as_secs_f64() * 1000.0
                );
                tracing::warn!(phase = "ui_complete", kind = unlock_dbg.kind, result = "error", elapsed_ms = elapsed_ms.as_str(), error = %err, "unlock.phase");
            }
            app.set_passkey_deriving = false;
            app.set_passkey_error = format!("Failed: {err}");
        }
        BackgroundEvent::ModelFetched(Ok(name)) => {
            tracing::info!(result = "ok", name = %name, "api.model");
            app.model_name = Some(name);
            if app.focus == Focus::LoadingDialog {
                app.focus = Focus::Input;
            }
        }
        BackgroundEvent::ModelFetched(Err(err)) => {
            tracing::error!(result = "error", error = %err, "api.model");
            app.model_name = Some("Backend connection failure".to_owned());
            app.api_available = false;
            app.api_error = err;
            match app.focus {
                Focus::PasskeyDialog | Focus::SetPasskeyDialog => {}
                _ => {
                    app.focus = Focus::ApiErrorDialog;
                }
            }
        }
        BackgroundEvent::ServerContextSize(size) => {
            const MIN_SERVER_CONTEXT_SIZE: usize = 1024;
            if size < MIN_SERVER_CONTEXT_SIZE {
                tracing::warn!(
                    n_ctx = size,
                    min = MIN_SERVER_CONTEXT_SIZE,
                    "api.context_size_ignored"
                );
            } else {
                let local_limit = app.context_mgr.token_limit();
                if size > local_limit {
                    tracing::warn!(
                        server_n_ctx = size,
                        local_limit = local_limit,
                        "api.context_size_clamped"
                    );
                }
                app.context_mgr.set_token_limit(size.min(local_limit));
            }
        }
        BackgroundEvent::TokenizerReloaded(token_counter) => {
            let is_heuristic = token_counter.is_heuristic();
            app.token_counter = token_counter;
            app.invalidate_prompt_cache();
            if is_heuristic {
                app.set_status(
                    "token counts approximate — llama.cpp /tokenize unavailable".to_owned(),
                    StatusLevel::Warning,
                );
            }
        }
        BackgroundEvent::TokenCountReady(update) => {
            tracing::trace!(
                key = update.key,
                result_is_ok = update.result.is_ok(),
                "tokenizer.update"
            );
            app.token_counter.apply_update(update);
            app.invalidate_prompt_cache();
        }
        BackgroundEvent::TemplateMatch { .. }
        | BackgroundEvent::EndpointChanged
        | BackgroundEvent::DangerOpComplete(_, _) => {}
    }
}
