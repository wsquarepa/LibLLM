use std::sync::Arc;

use tokio::sync::mpsc;

use crate::crypto::DerivedKey;
use crate::session::SaveMode;

use super::{App, BackgroundEvent, Focus, StatusLevel};

#[derive(Clone, Copy)]
pub(super) enum MaintenanceJob {
    CharacterPngImport,
    PlaintextCardEncryption,
    WorldbookNormalization,
}

pub(super) struct MaintenanceUpdate {
    pub(super) job: MaintenanceJob,
    pub(super) changed_count: usize,
    pub(super) warnings: Vec<String>,
}

impl MaintenanceJob {
    fn label(self) -> &'static str {
        match self {
            Self::CharacterPngImport => "character PNG import",
            Self::PlaintextCardEncryption => "plaintext card encryption",
            Self::WorldbookNormalization => "worldbook normalization",
        }
    }
}

pub(super) fn spawn_startup_maintenance(
    save_mode: &SaveMode,
    bg_tx: &mpsc::Sender<BackgroundEvent>,
) {
    match save_mode {
        SaveMode::Plaintext(_) => {
            spawn_character_png_import(None, bg_tx);
            spawn_worldbook_normalization(None, bg_tx);
        }
        SaveMode::Encrypted { key, .. } => {
            spawn_unlocked_maintenance(key.clone(), bg_tx);
        }
        SaveMode::None | SaveMode::PendingPasskey(_) => {}
    }
}

pub(super) fn spawn_unlocked_maintenance(
    key: Arc<DerivedKey>,
    bg_tx: &mpsc::Sender<BackgroundEvent>,
) {
    spawn_character_png_import(Some(key.clone()), bg_tx);
    spawn_worldbook_normalization(Some(key.clone()), bg_tx);
    spawn_plaintext_card_encryption(key, bg_tx);
}

pub(super) fn handle_finished(update: MaintenanceUpdate, app: &mut App) {
    let warnings = update.warnings;

    match update.job {
        MaintenanceJob::CharacterPngImport | MaintenanceJob::PlaintextCardEncryption => {
            if update.changed_count > 0 && matches!(app.focus, Focus::CharacterDialog) {
                reload_character_picker(app);
            }
        }
        MaintenanceJob::WorldbookNormalization => {
            if update.changed_count > 0 {
                app.invalidate_worldbook_cache();
                if matches!(app.focus, Focus::WorldbookDialog) {
                    reload_worldbook_picker(app);
                }
            }
        }
    }

    if warnings.is_empty() {
        return;
    }

    for warning in &warnings {
        crate::debug_log::log("maintenance.warning", warning);
    }

    let message = if warnings.len() == 1 {
        warnings[0].clone()
    } else {
        format!("{} (and {} more warnings)", warnings[0], warnings.len() - 1)
    };
    app.set_status(message, StatusLevel::Warning);
}

fn spawn_character_png_import(key: Option<Arc<DerivedKey>>, bg_tx: &mpsc::Sender<BackgroundEvent>) {
    spawn_job(MaintenanceJob::CharacterPngImport, bg_tx, move || {
        let report = crate::character::auto_import_png_cards(
            &crate::config::characters_dir(),
            key.as_deref(),
        );
        MaintenanceUpdate {
            job: MaintenanceJob::CharacterPngImport,
            changed_count: report.imported_count,
            warnings: report.warnings,
        }
    });
}

fn spawn_worldbook_normalization(
    key: Option<Arc<DerivedKey>>,
    bg_tx: &mpsc::Sender<BackgroundEvent>,
) {
    spawn_job(MaintenanceJob::WorldbookNormalization, bg_tx, move || {
        let report =
            crate::worldinfo::normalize_worldbooks(&crate::config::worldinfo_dir(), key.as_deref());
        MaintenanceUpdate {
            job: MaintenanceJob::WorldbookNormalization,
            changed_count: report.rewritten_count,
            warnings: report.warnings,
        }
    });
}

fn spawn_plaintext_card_encryption(key: Arc<DerivedKey>, bg_tx: &mpsc::Sender<BackgroundEvent>) {
    spawn_job(MaintenanceJob::PlaintextCardEncryption, bg_tx, move || {
        let report =
            crate::character::encrypt_plaintext_cards(&crate::config::characters_dir(), &key);
        MaintenanceUpdate {
            job: MaintenanceJob::PlaintextCardEncryption,
            changed_count: report.encrypted_count,
            warnings: report.warnings,
        }
    });
}

fn spawn_job<F>(job: MaintenanceJob, bg_tx: &mpsc::Sender<BackgroundEvent>, work: F)
where
    F: FnOnce() -> MaintenanceUpdate + Send + 'static,
{
    let tx = bg_tx.clone();
    crate::debug_log::log("maintenance.schedule", job.label());
    tokio::spawn(async move {
        let update = match tokio::task::spawn_blocking(work).await {
            Ok(update) => update,
            Err(err) => MaintenanceUpdate {
                job,
                changed_count: 0,
                warnings: vec![format!("{} failed: {err}", job.label())],
            },
        };
        crate::debug_log::log(
            "maintenance.complete",
            &format!(
                "{} changed={} warnings={}",
                update.job.label(),
                update.changed_count,
                update.warnings.len()
            ),
        );
        let _ = tx.send(BackgroundEvent::MaintenanceFinished(update)).await;
    });
}

fn reload_character_picker(app: &mut App) {
    let selected_slug = app.character_slugs.get(app.character_selected).cloned();
    let cards = crate::character::list_cards(&crate::config::characters_dir(), app.save_mode.key());

    app.character_names = cards.iter().map(|card| card.name.clone()).collect();
    app.character_slugs = cards.into_iter().map(|card| card.slug).collect();
    app.character_selected = selected_slug
        .and_then(|slug| {
            app.character_slugs
                .iter()
                .position(|existing| existing == &slug)
        })
        .unwrap_or(0)
        .min(app.character_slugs.len().saturating_sub(1));
}

fn reload_worldbook_picker(app: &mut App) {
    let selected_name = app.worldbook_list.get(app.worldbook_selected).cloned();
    let books =
        crate::worldinfo::list_worldbooks(&crate::config::worldinfo_dir(), app.save_mode.key());

    app.worldbook_list = books.into_iter().map(|book| book.name).collect();
    app.worldbook_selected = selected_name
        .and_then(|name| {
            app.worldbook_list
                .iter()
                .position(|existing| existing == &name)
        })
        .unwrap_or(0)
        .min(app.worldbook_list.len().saturating_sub(1));
}
