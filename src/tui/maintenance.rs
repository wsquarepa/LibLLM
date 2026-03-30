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
    SystemPromptSetup,
    PlaintextPromptEncryption,
    PersonaMigration,
    PlaintextPersonaEncryption,
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
            Self::SystemPromptSetup => "system prompt setup",
            Self::PlaintextPromptEncryption => "plaintext prompt encryption",
            Self::PersonaMigration => "persona migration",
            Self::PlaintextPersonaEncryption => "plaintext persona encryption",
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
            spawn_system_prompt_setup(None, bg_tx);
            spawn_persona_migration(None, bg_tx);
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
    spawn_system_prompt_setup(Some(key.clone()), bg_tx);
    spawn_persona_migration(Some(key.clone()), bg_tx);
    spawn_plaintext_card_encryption(key.clone(), bg_tx);
    spawn_plaintext_prompt_encryption(key.clone(), bg_tx);
    spawn_plaintext_persona_encryption(key, bg_tx);
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
        MaintenanceJob::SystemPromptSetup | MaintenanceJob::PlaintextPromptEncryption => {}
        MaintenanceJob::PersonaMigration | MaintenanceJob::PlaintextPersonaEncryption => {
            if update.changed_count > 0 && matches!(app.focus, Focus::PersonaDialog) {
                reload_persona_picker(app);
            }
        }
    }

    if warnings.is_empty() {
        return;
    }

    for warning in &warnings {
        crate::debug_log::log_kv(
            "maintenance.warning",
            &[
                crate::debug_log::field("phase", "warning"),
                crate::debug_log::field("message", warning),
            ],
        );
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
        let result = crate::migration::migrate_worldbook_normalization(key.as_deref());
        MaintenanceUpdate {
            job: MaintenanceJob::WorldbookNormalization,
            changed_count: result.changed_count,
            warnings: result.warnings,
        }
    });
}

fn spawn_plaintext_card_encryption(key: Arc<DerivedKey>, bg_tx: &mpsc::Sender<BackgroundEvent>) {
    spawn_job(MaintenanceJob::PlaintextCardEncryption, bg_tx, move || {
        let result = crate::migration::migrate_encrypt_plaintext_cards(&key);
        MaintenanceUpdate {
            job: MaintenanceJob::PlaintextCardEncryption,
            changed_count: result.changed_count,
            warnings: result.warnings,
        }
    });
}

fn spawn_system_prompt_setup(
    key: Option<Arc<DerivedKey>>,
    bg_tx: &mpsc::Sender<BackgroundEvent>,
) {
    spawn_job(MaintenanceJob::SystemPromptSetup, bg_tx, move || {
        crate::migration::migrate_system_prompts_from_config(key.as_deref());
        crate::system_prompt::ensure_builtin_prompts(
            &crate::config::system_prompts_dir(),
            key.as_deref(),
        );
        MaintenanceUpdate {
            job: MaintenanceJob::SystemPromptSetup,
            changed_count: 0,
            warnings: Vec::new(),
        }
    });
}

fn spawn_plaintext_prompt_encryption(
    key: Arc<DerivedKey>,
    bg_tx: &mpsc::Sender<BackgroundEvent>,
) {
    spawn_job(MaintenanceJob::PlaintextPromptEncryption, bg_tx, move || {
        let result = crate::migration::migrate_encrypt_plaintext_prompts(&key);
        MaintenanceUpdate {
            job: MaintenanceJob::PlaintextPromptEncryption,
            changed_count: result.changed_count,
            warnings: result.warnings,
        }
    });
}

fn spawn_persona_migration(
    key: Option<Arc<DerivedKey>>,
    bg_tx: &mpsc::Sender<BackgroundEvent>,
) {
    spawn_job(MaintenanceJob::PersonaMigration, bg_tx, move || {
        let result = crate::migration::migrate_personas_from_config(key.as_deref());
        MaintenanceUpdate {
            job: MaintenanceJob::PersonaMigration,
            changed_count: result.changed_count,
            warnings: result.warnings,
        }
    });
}

fn spawn_plaintext_persona_encryption(
    key: Arc<DerivedKey>,
    bg_tx: &mpsc::Sender<BackgroundEvent>,
) {
    spawn_job(MaintenanceJob::PlaintextPersonaEncryption, bg_tx, move || {
        let result = crate::migration::migrate_encrypt_plaintext_personas(&key);
        MaintenanceUpdate {
            job: MaintenanceJob::PlaintextPersonaEncryption,
            changed_count: result.changed_count,
            warnings: result.warnings,
        }
    });
}

fn spawn_job<F>(job: MaintenanceJob, bg_tx: &mpsc::Sender<BackgroundEvent>, work: F)
where
    F: FnOnce() -> MaintenanceUpdate + Send + 'static,
{
    let tx = bg_tx.clone();
    crate::debug_log::log_kv(
        "maintenance.schedule",
        &[
            crate::debug_log::field("job", job.label()),
            crate::debug_log::field("phase", "schedule"),
        ],
    );
    tokio::spawn(async move {
        let update = match tokio::task::spawn_blocking(work).await {
            Ok(update) => update,
            Err(err) => MaintenanceUpdate {
                job,
                changed_count: 0,
                warnings: vec![format!("{} failed: {err}", job.label())],
            },
        };
        crate::debug_log::log_kv(
            "maintenance.complete",
            &[
                crate::debug_log::field("job", update.job.label()),
                crate::debug_log::field("changed", update.changed_count),
                crate::debug_log::field("warnings", update.warnings.len()),
            ],
        );
        let _ = tx.send(BackgroundEvent::MaintenanceFinished(update)).await;
    });
}

pub(in crate::tui) fn reload_character_picker(app: &mut App) {
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

pub(in crate::tui) fn reload_worldbook_picker(app: &mut App) {
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

pub(in crate::tui) fn reload_persona_picker(app: &mut App) {
    let selected_name = app.persona_list.get(app.persona_selected).cloned();
    let personas = crate::persona::list_personas(
        &crate::config::personas_dir(),
        app.save_mode.key(),
    );

    app.persona_list = personas.into_iter().map(|p| p.name).collect();
    app.persona_selected = selected_name
        .and_then(|name| {
            app.persona_list
                .iter()
                .position(|existing| existing == &name)
        })
        .unwrap_or(0)
        .min(app.persona_list.len().saturating_sub(1));
}

pub(in crate::tui) fn reload_system_prompt_picker(app: &mut App) {
    let selected_name = app.system_prompt_list.get(app.system_prompt_selected).cloned();
    let prompts = crate::system_prompt::list_prompts(
        &crate::config::system_prompts_dir(),
        app.save_mode.key(),
    );

    app.system_prompt_list = prompts.into_iter().map(|p| p.name).collect();
    app.system_prompt_selected = selected_name
        .and_then(|name| {
            app.system_prompt_list
                .iter()
                .position(|existing| existing == &name)
        })
        .unwrap_or(0)
        .min(app.system_prompt_list.len().saturating_sub(1));
}
