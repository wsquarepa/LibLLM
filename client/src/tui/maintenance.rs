//! Startup maintenance tasks and runtime picker state reloading.

use libllm::session::SaveMode;

use super::App;

pub(super) fn spawn_startup_maintenance(
    save_mode: &SaveMode,
    app: &App,
) {
    match save_mode {
        SaveMode::Database { .. } => {
            if let Some(ref db) = app.db {
                if let Err(e) = db.ensure_builtin_prompts() {
                    libllm::debug_log::log_kv(
                        "maintenance.warning",
                        &[
                            libllm::debug_log::field("phase", "ensure_builtins"),
                            libllm::debug_log::field("error", &e),
                        ],
                    );
                }
            }
        }
        SaveMode::None | SaveMode::PendingPasskey { .. } => {}
    }
}

pub(in crate::tui) fn reload_character_picker(app: &mut App) {
    let selected_slug = app.character_slugs.get(app.character_selected).cloned();
    let (names, slugs) = match app.db.as_ref().and_then(|db| db.list_characters().ok()) {
        Some(chars) => {
            let names: Vec<String> = chars.iter().map(|(_, name)| name.clone()).collect();
            let slugs: Vec<String> = chars.into_iter().map(|(slug, _)| slug).collect();
            (names, slugs)
        }
        None => (Vec::new(), Vec::new()),
    };

    app.character_names = names;
    app.character_slugs = slugs;
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
    let books = match app.db.as_ref().and_then(|db| db.list_worldbooks().ok()) {
        Some(wbs) => wbs.into_iter().map(|(_, name)| name).collect(),
        None => Vec::new(),
    };

    app.worldbook_list = books;
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
    let selected_slug = app.persona_slugs.get(app.persona_selected).cloned();
    let personas = app
        .db
        .as_ref()
        .and_then(|db| db.list_personas().ok())
        .unwrap_or_default();

    app.persona_names = personas.iter().map(|(_, name)| name.clone()).collect();
    app.persona_slugs = personas.into_iter().map(|(slug, _)| slug).collect();
    app.persona_selected = selected_slug
        .and_then(|slug| {
            app.persona_slugs
                .iter()
                .position(|existing| existing == &slug)
        })
        .unwrap_or(0)
        .min(app.persona_slugs.len().saturating_sub(1));
}

pub(in crate::tui) fn reload_system_prompt_picker(app: &mut App) {
    let selected_name = app
        .system_prompt_list
        .get(app.system_prompt_selected)
        .cloned();
    let prompts = match app.db.as_ref().and_then(|db| db.list_prompts().ok()) {
        Some(ps) => ps.into_iter().map(|e| e.name).collect(),
        None => Vec::new(),
    };

    app.system_prompt_list = prompts;
    app.system_prompt_selected = selected_name
        .and_then(|name| {
            app.system_prompt_list
                .iter()
                .position(|existing| existing == &name)
        })
        .unwrap_or(0)
        .min(app.system_prompt_list.len().saturating_sub(1));
}
