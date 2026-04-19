//! Background event processing for passkey derivation and model name resolution.

use libllm::db::Database;
use libllm::session::{self, SaveMode};

use crate::tui::business;
use crate::tui::types::{BackgroundEvent, Focus, SaveTrigger, StatusLevel};

use super::App;

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
                    if let Err(err) = app.flush_session_save(SaveTrigger::Unlock) {
                        app.set_status(format!("Save error: {err}"), StatusLevel::Error);
                    }
                    app.invalidate_worldbook_cache();
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
                        if let Err(err) = app.flush_session_save(SaveTrigger::Unlock) {
                            app.set_status(format!("Save error: {err}"), StatusLevel::Error);
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
                    match db.rekey(&new_key) {
                        Ok(()) => {
                            app.passkey_changed = true;
                            app.should_quit = true;
                        }
                        Err(err) => {
                            app.set_status(
                                format!("Failed to change passkey: {err}"),
                                StatusLevel::Error,
                            );
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
    }
}
