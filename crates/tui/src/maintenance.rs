//! Startup maintenance tasks and runtime picker state reloading.

use libllm_core::session::SaveMode;

use super::App;

pub(super) fn spawn_startup_maintenance(save_mode: &SaveMode, app: &App) {
    match save_mode {
        SaveMode::Database { .. } => {
            if let Some(ref db) = app.db
                && let Err(e) = db.ensure_builtin_prompts()
            {
                tracing::warn!(phase = "ensure_builtins", error = %e, "maintenance.warning");
            }
        }
        SaveMode::None | SaveMode::PendingPasskey { .. } => {}
    }
}

/// Index of `previous` in `list`, or 0 when absent, clamped to the last valid index
/// (0 for an empty list).
fn restore_selection(list: &[String], previous: Option<String>) -> usize {
    previous
        .and_then(|p| list.iter().position(|existing| *existing == p))
        .unwrap_or(0)
        .min(list.len().saturating_sub(1))
}

pub(crate) fn reload_character_picker(app: &mut App) {
    let selected_slug = app.character.slugs.get(app.character.selected).cloned();
    let (names, slugs) = match app.db.as_ref().and_then(|db| db.list_characters().ok()) {
        Some(chars) => {
            let names: Vec<String> = chars.iter().map(|(_, name)| name.clone()).collect();
            let slugs: Vec<String> = chars.into_iter().map(|(slug, _)| slug).collect();
            (names, slugs)
        }
        None => (Vec::new(), Vec::new()),
    };

    app.character.names = names;
    app.character.slugs = slugs;
    app.character.selected = restore_selection(&app.character.slugs, selected_slug);
}

pub(crate) fn reload_worldbook_picker(app: &mut App) {
    let selected_name = app.worldbook.list.get(app.worldbook.list_selected).cloned();
    let books = match app.db.as_ref().and_then(|db| db.list_worldbooks().ok()) {
        Some(wbs) => wbs.into_iter().map(|(_, name)| name).collect(),
        None => Vec::new(),
    };

    app.worldbook.list = books;
    app.worldbook.list_selected = restore_selection(&app.worldbook.list, selected_name);
}

pub(crate) fn reload_persona_picker(app: &mut App) {
    let selected_slug = app.persona.slugs.get(app.persona.selected).cloned();
    let personas = app
        .db
        .as_ref()
        .and_then(|db| db.list_personas().ok())
        .unwrap_or_default();

    app.persona.names = personas.iter().map(|(_, name)| name.clone()).collect();
    app.persona.slugs = personas.into_iter().map(|(slug, _)| slug).collect();
    app.persona.selected = restore_selection(&app.persona.slugs, selected_slug);
}

pub(crate) fn reload_system_prompt_picker(app: &mut App) {
    let selected_name = app
        .system_prompt
        .list
        .get(app.system_prompt.selected)
        .cloned();
    let prompts = match app.db.as_ref().and_then(|db| db.list_prompts().ok()) {
        Some(ps) => ps.into_iter().map(|e| e.name).collect(),
        None => Vec::new(),
    };

    app.system_prompt.list = prompts;
    app.system_prompt.selected = restore_selection(&app.system_prompt.list, selected_name);
}

#[cfg(test)]
mod tests {
    use super::restore_selection;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn restore_selection_keeps_previous_when_present() {
        assert_eq!(
            restore_selection(&names(&["a", "b", "c"]), Some("c".to_owned())),
            2
        );
    }

    #[test]
    fn restore_selection_falls_back_to_first_when_absent() {
        assert_eq!(
            restore_selection(&names(&["a", "b"]), Some("zzz".to_owned())),
            0
        );
        assert_eq!(restore_selection(&names(&["a", "b"]), None), 0);
    }

    #[test]
    fn restore_selection_is_zero_for_empty_list() {
        assert_eq!(restore_selection(&[], Some("a".to_owned())), 0);
    }
}
