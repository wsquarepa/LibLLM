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
            if let Some(debug) = app.unlock_debug.take() {
                libllm::debug_log::log_kv(
                    "unlock.phase",
                    &[
                        libllm::debug_log::field("phase", "ui_complete"),
                        libllm::debug_log::field("kind", debug.kind),
                        libllm::debug_log::field("result", "ok"),
                        libllm::debug_log::field(
                            "elapsed_ms",
                            format!("{:.3}", debug.started_at.elapsed().as_secs_f64() * 1000.0),
                        ),
                    ],
                );
            }
            match Database::open(&db_path, Some(&key)) {
                Ok(db) => {
                    if let Err(e) = db.ensure_builtin_prompts() {
                        app.set_status(format!("Warning: {e}"), StatusLevel::Warning);
                    }
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
            if let Some(debug) = app.unlock_debug.take() {
                libllm::debug_log::log_kv(
                    "unlock.phase",
                    &[
                        libllm::debug_log::field("phase", "ui_complete"),
                        libllm::debug_log::field("kind", debug.kind),
                        libllm::debug_log::field("result", "error"),
                        libllm::debug_log::field(
                            "elapsed_ms",
                            format!("{:.3}", debug.started_at.elapsed().as_secs_f64() * 1000.0),
                        ),
                        libllm::debug_log::field("error", &err),
                    ],
                );
            }
            app.passkey_deriving = false;
            app.passkey_error = format!("Failed: {err}");
            app.resolved_passkey = None;
        }
        BackgroundEvent::PasskeySet(new_key) => {
            if let Some(debug) = app.unlock_debug.take() {
                libllm::debug_log::log_kv(
                    "unlock.phase",
                    &[
                        libllm::debug_log::field("phase", "ui_complete"),
                        libllm::debug_log::field("kind", debug.kind),
                        libllm::debug_log::field("result", "ok"),
                        libllm::debug_log::field(
                            "elapsed_ms",
                            format!("{:.3}", debug.started_at.elapsed().as_secs_f64() * 1000.0),
                        ),
                    ],
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
            if let Some(debug) = app.unlock_debug.take() {
                libllm::debug_log::log_kv(
                    "unlock.phase",
                    &[
                        libllm::debug_log::field("phase", "ui_complete"),
                        libllm::debug_log::field("kind", debug.kind),
                        libllm::debug_log::field("result", "error"),
                        libllm::debug_log::field(
                            "elapsed_ms",
                            format!("{:.3}", debug.started_at.elapsed().as_secs_f64() * 1000.0),
                        ),
                        libllm::debug_log::field("error", &err),
                    ],
                );
            }
            app.set_passkey_deriving = false;
            app.set_passkey_error = format!("Failed: {err}");
        }
        BackgroundEvent::ModelFetched(Ok(name)) => {
            app.model_name = Some(name);
            if app.focus == Focus::LoadingDialog {
                app.focus = Focus::Input;
            }
        }
        BackgroundEvent::ModelFetched(Err(err)) => {
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
            app.context_mgr.set_token_limit(size);
        }
    }
}
