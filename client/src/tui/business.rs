//! Business logic helpers for system prompt assembly, worldbook injection, and config management.

use libllm::db::Database;
use libllm::session::{Message, Role, SaveMode, Session, SessionEntry};
use libllm::worldinfo::{ActivatedEntry, RuntimeWorldBook};

use super::App;

const SIDEBAR_PREVIEW_CHARS: usize = 28;

pub fn non_empty(s: &str) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s.to_owned())
    }
}

pub use libllm::template::apply_template_vars;

/// Assembles the final system prompt from session, builtin defaults, and persona.
///
/// Falls back to the builtin assistant or roleplay prompt when the session has no explicit
/// system prompt. Appends persona description when a character session has a persona set.
/// Returns `None` when neither a base prompt nor a persona is available.
pub fn build_effective_system_prompt(
    session: &Session,
    db: Option<&Database>,
) -> Option<String> {
    let is_character = session.character.is_some();

    let session_prompt = session.system_prompt.as_deref().unwrap_or("");

    let builtin_name = if is_character {
        libllm::system_prompt::BUILTIN_ROLEPLAY
    } else {
        libllm::system_prompt::BUILTIN_ASSISTANT
    };
    let resolved_default = db.and_then(|db| {
        db.load_prompt(builtin_name).ok().map(|p| p.content)
    });
    let config_default = resolved_default.as_deref().unwrap_or("");

    let base = if session_prompt.is_empty() {
        config_default
    } else {
        session_prompt
    };

    let persona = session.persona.as_ref().and_then(|name| {
        db.and_then(|db| db.load_persona(name).ok())
    });

    let has_persona = is_character && persona.is_some();

    if base.is_empty() && !has_persona {
        return None;
    }

    let mut parts: Vec<String> = Vec::new();
    if !base.is_empty() {
        parts.push(base.to_owned());
    }
    if has_persona {
        let pf = persona.as_ref().unwrap();
        let name = if pf.name.is_empty() {
            "the user"
        } else {
            &pf.name
        };
        let mut persona_line = format!("The user's name is {name}.");
        if !pf.persona.is_empty() {
            persona_line.push_str(&format!(" {}", pf.persona));
        }
        parts.push(persona_line);
    }

    let mut result = parts.join("\n\n");
    if is_character {
        let char_name = session.character.as_deref().unwrap_or("");
        let user_name = persona
            .as_ref()
            .and_then(|p| {
                if p.name.is_empty() {
                    None
                } else {
                    Some(p.name.as_str())
                }
            })
            .unwrap_or("User");
        result = apply_template_vars(&result, char_name, user_name);
    }

    Some(result)
}

pub fn enabled_worldbook_names(session: &Session, cfg: &libllm::config::Config) -> Vec<String> {
    let mut enabled = cfg.worldbooks.clone();
    for name in &session.worldbooks {
        if !enabled.iter().any(|existing| existing == name) {
            enabled.push(name.clone());
        }
    }
    enabled
}

pub fn load_runtime_worldbooks(
    enabled: &[String],
    db: Option<&Database>,
) -> Vec<RuntimeWorldBook> {
    libllm::debug_log::timed_kv(
        "worldbook.runtime",
        &[
            libllm::debug_log::field("phase", "hydrate"),
            libllm::debug_log::field("enabled_count", enabled.len()),
        ],
        || {
            let Some(db) = db else { return Vec::new() };
            enabled
                .iter()
                .filter_map(|wb_name| {
                    db.load_worldbook(wb_name)
                        .ok()
                        .map(|wb| RuntimeWorldBook::from_worldbook(&wb))
                })
                .collect()
        },
    )
}

/// Scans all loaded worldbooks against recent messages and injects activated entries as system
/// messages at the appropriate depth positions in the message list.
pub fn inject_loaded_worldbook_entries(
    session: &Session,
    messages: &[&Message],
    user_name: &str,
    worldbooks: &[RuntimeWorldBook],
) -> Vec<Message> {
    if session.character.is_none() || worldbooks.is_empty() {
        return messages.iter().map(|m| (*m).clone()).collect();
    }

    let char_name = session.character.as_deref().unwrap_or("");
    let msg_texts: Vec<&str> = messages.iter().map(|m| m.content.as_str()).collect();

    let mut all_activated: Vec<ActivatedEntry> = worldbooks
        .iter()
        .flat_map(|wb| libllm::worldinfo::scan_runtime_entries(wb, &msg_texts))
        .collect();

    if all_activated.is_empty() {
        return messages.iter().map(|m| (*m).clone()).collect();
    }

    all_activated.sort_by_key(|e| e.order);

    let mut result: Vec<Message> = messages.iter().map(|m| (*m).clone()).collect();
    let len = result.len();

    let mut insertions: Vec<(usize, usize, Message)> = all_activated
        .into_iter()
        .enumerate()
        .map(|(i, entry)| {
            let content = apply_template_vars(&entry.content, char_name, user_name);
            let pos = if entry.depth == 0 || entry.depth >= len {
                0
            } else {
                len - entry.depth
            };
            (pos, i, Message::new(Role::System, content))
        })
        .collect();

    insertions.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));

    for (pos, _, msg) in insertions {
        result.insert(pos, msg);
    }

    result
}

pub fn replace_template_vars(
    session: &Session,
    messages: Vec<Message>,
    user_name: &str,
) -> Vec<Message> {
    if session.character.is_none() {
        return messages;
    }

    let char_name = session.character.as_deref().unwrap_or("");

    messages
        .into_iter()
        .map(|mut msg| {
            msg.content = apply_template_vars(&msg.content, char_name, user_name);
            msg
        })
        .collect()
}

pub fn load_tabbed_config_sections(
    cfg: &libllm::config::Config,
    overrides: &crate::cli::CliOverrides,
) -> Vec<Vec<String>> {
    let defaults = libllm::sampling::SamplingParams::default();

    let general = vec![
        overrides
            .api_url
            .as_deref()
            .or(cfg.api_url.as_deref())
            .unwrap_or(libllm::config::Config::default().api_url())
            .to_owned(),
        cfg.template_preset.as_deref().unwrap_or("Default").to_owned(),
        overrides
            .template
            .as_deref()
            .or(cfg.instruct_preset.as_deref())
            .unwrap_or("Mistral V3-Tekken")
            .to_owned(),
        cfg.reasoning_preset.as_deref().unwrap_or("OFF").to_owned(),
        if overrides.tls_skip_verify {
            "true".to_owned()
        } else {
            cfg.tls_skip_verify.to_string()
        },
        cfg.debug_log.to_string(),
    ];

    let sampling = vec![
        overrides
            .sampling
            .temperature
            .or(cfg.sampling.temperature)
            .unwrap_or(defaults.temperature)
            .to_string(),
        overrides
            .sampling
            .top_k
            .or(cfg.sampling.top_k)
            .unwrap_or(defaults.top_k)
            .to_string(),
        overrides
            .sampling
            .top_p
            .or(cfg.sampling.top_p)
            .unwrap_or(defaults.top_p)
            .to_string(),
        overrides
            .sampling
            .min_p
            .or(cfg.sampling.min_p)
            .unwrap_or(defaults.min_p)
            .to_string(),
        overrides
            .sampling
            .repeat_last_n
            .or(cfg.sampling.repeat_last_n)
            .unwrap_or(defaults.repeat_last_n)
            .to_string(),
        overrides
            .sampling
            .repeat_penalty
            .or(cfg.sampling.repeat_penalty)
            .unwrap_or(defaults.repeat_penalty)
            .to_string(),
        overrides
            .sampling
            .max_tokens
            .or(cfg.sampling.max_tokens)
            .unwrap_or(defaults.max_tokens)
            .to_string(),
    ];

    let backup = vec![
        cfg.backup.enabled.to_string(),
        cfg.backup.keep_all_days.to_string(),
        cfg.backup.keep_daily_days.to_string(),
        cfg.backup.keep_weekly_days.to_string(),
        cfg.backup.rebase_threshold_percent.to_string(),
        cfg.backup.rebase_hard_ceiling.to_string(),
    ];

    let summarization = vec![
        cfg.summarization.enabled.to_string(),
        cfg.summarization.api_url.clone().unwrap_or_default(),
        cfg.summarization.context_size.to_string(),
        cfg.summarization.trigger_threshold.to_string(),
        cfg.summarization.prompt.clone(),
    ];

    vec![general, sampling, backup, summarization]
}

pub fn config_locked_fields_by_section(
    overrides: &crate::cli::CliOverrides,
) -> Vec<Vec<usize>> {
    let mut general: Vec<usize> = Vec::new();
    let mut sampling: Vec<usize> = Vec::new();
    if overrides.api_url.is_some() {
        general.push(0);
    }
    if overrides.template.is_some() {
        general.push(2);
    }
    if overrides.tls_skip_verify {
        general.push(4);
    }
    if overrides.sampling.temperature.is_some() {
        sampling.push(0);
    }
    if overrides.sampling.top_k.is_some() {
        sampling.push(1);
    }
    if overrides.sampling.top_p.is_some() {
        sampling.push(2);
    }
    if overrides.sampling.min_p.is_some() {
        sampling.push(3);
    }
    if overrides.sampling.repeat_last_n.is_some() {
        sampling.push(4);
    }
    if overrides.sampling.repeat_penalty.is_some() {
        sampling.push(5);
    }
    if overrides.sampling.max_tokens.is_some() {
        sampling.push(6);
    }
    vec![general, sampling, Vec::new(), Vec::new()]
}

pub fn apply_tabbed_config_fields(
    sections: &[Vec<String>],
    existing: libllm::config::Config,
    overrides: &crate::cli::CliOverrides,
) -> anyhow::Result<()> {
    let locked = config_locked_fields_by_section(overrides);
    let general = &sections[0];
    let sampling = &sections[1];
    let backup = &sections[2];
    let summarization = &sections[3];

    let api_url = if locked[0].contains(&0) {
        existing.api_url.clone()
    } else {
        non_empty(&general[0])
    };
    let template_preset = non_empty(&general[1]);
    let instruct_preset = if locked[0].contains(&2) {
        existing.instruct_preset.clone()
    } else {
        non_empty(&general[2])
    };
    let reasoning_preset = non_empty(&general[3]).filter(|v| !v.eq_ignore_ascii_case("OFF"));
    let tls_skip_verify = if locked[0].contains(&4) {
        existing.tls_skip_verify
    } else {
        general[4] == "true"
    };
    let debug_log = general[5] == "true";

    let temperature = if locked[1].contains(&0) {
        existing.sampling.temperature
    } else {
        parse_f64_clamped(&sampling[0], 0.0, 2.0)
    };
    let top_k = if locked[1].contains(&1) {
        existing.sampling.top_k
    } else {
        parse_i64_clamped(&sampling[1], 1, 100)
    };
    let top_p = if locked[1].contains(&2) {
        existing.sampling.top_p
    } else {
        parse_f64_clamped(&sampling[2], 0.0, 1.0)
    };
    let min_p = if locked[1].contains(&3) {
        existing.sampling.min_p
    } else {
        parse_f64_clamped(&sampling[3], 0.0, 1.0)
    };
    let repeat_last_n = if locked[1].contains(&4) {
        existing.sampling.repeat_last_n
    } else {
        parse_i64_clamped(&sampling[4], -1, 32767)
    };
    let repeat_penalty = if locked[1].contains(&5) {
        existing.sampling.repeat_penalty
    } else {
        parse_f64_clamped(&sampling[5], 0.0, 2.0)
    };
    let max_tokens = if locked[1].contains(&6) {
        existing.sampling.max_tokens
    } else {
        parse_i64_clamped(&sampling[6], -1, 32767)
    };

    let backup_cfg = libllm::config::BackupConfig {
        enabled: backup[0] == "true",
        keep_all_days: parse_u32_clamped(&backup[1], 0, 3650),
        keep_daily_days: parse_u32_clamped(&backup[2], 0, 3650),
        keep_weekly_days: parse_u32_clamped(&backup[3], 0, 3650),
        rebase_threshold_percent: parse_u32_clamped(&backup[4], 0, 100),
        rebase_hard_ceiling: parse_u32_clamped(&backup[5], 0, 100),
    };

    let summarization_api_url = if summarization[1].trim().is_empty() {
        None
    } else {
        Some(summarization[1].clone())
    };
    let summarization_cfg = libllm::config::SummarizationConfig {
        enabled: summarization[0] == "true",
        api_url: summarization_api_url,
        context_size: parse_usize_clamped(&summarization[2], 512, 131072),
        trigger_threshold: parse_usize_clamped(&summarization[3], 1, 100),
        prompt: summarization[4].clone(),
    };

    let cfg = libllm::config::Config {
        api_url,
        template_preset,
        instruct_preset,
        reasoning_preset,
        sampling: libllm::sampling::SamplingOverrides {
            temperature,
            top_k,
            top_p,
            min_p,
            repeat_last_n,
            repeat_penalty,
            max_tokens,
        },
        worldbooks: existing.worldbooks,
        tls_skip_verify,
        debug_log,
        default_persona: existing.default_persona,
        macros: existing.macros,
        theme: existing.theme,
        theme_colors: existing.theme_colors,
        backup: backup_cfg,
        summarization: summarization_cfg,
    };

    libllm::config::save(&cfg)
}

fn parse_f64_clamped(s: &str, min: f64, max: f64) -> Option<f64> {
    s.parse::<f64>().ok().map(|v| v.clamp(min, max))
}

fn parse_i64_clamped(s: &str, min: i64, max: i64) -> Option<i64> {
    s.parse::<i64>().ok().map(|v| v.clamp(min, max))
}

fn parse_u32_clamped(value: &str, min: u32, max: u32) -> u32 {
    value
        .trim()
        .parse::<u32>()
        .ok()
        .map(|v| v.clamp(min, max))
        .unwrap_or(min)
}

fn parse_usize_clamped(value: &str, min: usize, max: usize) -> usize {
    value
        .trim()
        .parse::<usize>()
        .ok()
        .map(|v| v.clamp(min, max))
        .unwrap_or(min)
}

pub(super) fn apply_config(app: &mut App) {
    let cfg = libllm::config::load();
    let preset_name = app
        .cli_overrides
        .template
        .as_deref()
        .or(cfg.instruct_preset.as_deref())
        .unwrap_or("Mistral V3-Tekken");
    app.instruct_preset = libllm::preset::resolve_instruct_preset(preset_name);
    app.stop_tokens = app.instruct_preset.stop_tokens();
    app.sampling = libllm::sampling::SamplingParams::default()
        .with_overrides(&cfg.sampling)
        .with_overrides(&app.cli_overrides.sampling);

    let new_url = app.cli_overrides.api_url.as_deref().unwrap_or(cfg.api_url());
    let new_tls_skip = app.cli_overrides.tls_skip_verify || cfg.tls_skip_verify;
    app.client = libllm::client::ApiClient::new(new_url, new_tls_skip);
    app.model_name = None;
    app.api_available = true;
    app.api_error.clear();
    let client = app.client.clone();
    let tx = app.bg_tx.clone();
    tokio::spawn(async move {
        let result = client.fetch_model_name().await;
        let _ = tx.send(super::BackgroundEvent::ModelFetched(result)).await;
    });

    app.theme = super::theme::resolve_theme(&cfg);
    app.config = cfg;
    app.invalidate_worldbook_cache();
    app.invalidate_chat_cache();
}

pub fn build_theme_color_overrides(sections: &[Vec<String>]) -> libllm::config::ThemeColorOverrides {
    libllm::config::ThemeColorOverrides {
        user_message: non_empty(&sections[1][0]),
        assistant_message_fg: non_empty(&sections[1][1]),
        assistant_message_bg: non_empty(&sections[1][2]),
        system_message: non_empty(&sections[1][3]),
        dialogue: non_empty(&sections[1][4]),
        border_focused: non_empty(&sections[2][0]),
        border_unfocused: non_empty(&sections[2][1]),
        status_bar_fg: non_empty(&sections[2][2]),
        status_bar_bg: non_empty(&sections[2][3]),
        status_error_fg: non_empty(&sections[2][4]),
        status_error_bg: non_empty(&sections[2][5]),
        status_info_fg: non_empty(&sections[2][6]),
        status_info_bg: non_empty(&sections[2][7]),
        status_warning_fg: non_empty(&sections[2][8]),
        status_warning_bg: non_empty(&sections[2][9]),
        nav_cursor_fg: non_empty(&sections[3][0]),
        nav_cursor_bg: non_empty(&sections[3][1]),
        hover_bg: non_empty(&sections[3][2]),
        sidebar_highlight_fg: non_empty(&sections[3][3]),
        sidebar_highlight_bg: non_empty(&sections[3][4]),
        dimmed: non_empty(&sections[3][5]),
        command_picker_fg: non_empty(&sections[3][6]),
        command_picker_bg: non_empty(&sections[3][7]),
        streaming_indicator: non_empty(&sections[4][0]),
        api_unavailable: non_empty(&sections[4][1]),
        summary_indicator: non_empty(&sections[4][2]),
    }
}

pub fn apply_theme_color_sections(
    sections: &[Vec<String>],
    existing: libllm::config::Config,
) -> anyhow::Result<()> {
    let base_theme = sections[0][0].clone();

    let overrides = build_theme_color_overrides(sections);

    let any_set = [
        &overrides.user_message,
        &overrides.assistant_message_fg,
        &overrides.assistant_message_bg,
        &overrides.system_message,
        &overrides.dialogue,
        &overrides.border_focused,
        &overrides.border_unfocused,
        &overrides.status_bar_fg,
        &overrides.status_bar_bg,
        &overrides.status_error_fg,
        &overrides.status_error_bg,
        &overrides.status_info_fg,
        &overrides.status_info_bg,
        &overrides.status_warning_fg,
        &overrides.status_warning_bg,
        &overrides.nav_cursor_fg,
        &overrides.nav_cursor_bg,
        &overrides.hover_bg,
        &overrides.sidebar_highlight_fg,
        &overrides.sidebar_highlight_bg,
        &overrides.dimmed,
        &overrides.command_picker_fg,
        &overrides.command_picker_bg,
        &overrides.streaming_indicator,
        &overrides.api_unavailable,
        &overrides.summary_indicator,
    ]
    .iter()
    .any(|o| o.is_some());

    let cfg = libllm::config::Config {
        theme: Some(base_theme),
        theme_colors: if any_set { Some(overrides) } else { None },
        ..existing
    };

    libllm::config::save(&cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use libllm::config::Config;
    use libllm::session::Session;

    #[test]
    fn non_empty_with_content() {
        assert_eq!(non_empty("hello"), Some("hello".to_owned()));
    }

    #[test]
    fn non_empty_empty_string() {
        assert_eq!(non_empty(""), None);
    }

    #[test]
    fn non_empty_whitespace_only() {
        assert_eq!(non_empty("   "), None);
        assert_eq!(non_empty("\t\n"), None);
    }

    #[test]
    fn non_empty_with_surrounding_whitespace() {
        assert_eq!(non_empty("  hello  "), Some("  hello  ".to_owned()));
    }

    #[test]
    fn enabled_worldbook_names_session_only() {
        let mut session = Session::default();
        session.worldbooks = vec!["lore_a".to_owned(), "lore_b".to_owned()];
        let cfg = Config::default();

        let names = enabled_worldbook_names(&session, &cfg);
        assert_eq!(names, vec!["lore_a", "lore_b"]);
    }

    #[test]
    fn enabled_worldbook_names_config_only() {
        let session = Session::default();
        let cfg = Config {
            worldbooks: vec!["cfg_lore".to_owned()],
            ..Config::default()
        };

        let names = enabled_worldbook_names(&session, &cfg);
        assert_eq!(names, vec!["cfg_lore"]);
    }

    #[test]
    fn enabled_worldbook_names_merged_dedup() {
        let mut session = Session::default();
        session.worldbooks = vec!["shared".to_owned(), "session_only".to_owned()];
        let cfg = Config {
            worldbooks: vec!["shared".to_owned(), "cfg_only".to_owned()],
            ..Config::default()
        };

        let names = enabled_worldbook_names(&session, &cfg);
        assert!(names.contains(&"shared".to_owned()));
        assert!(names.contains(&"cfg_only".to_owned()));
        assert!(names.contains(&"session_only".to_owned()));
        assert_eq!(
            names.iter().filter(|n| *n == "shared").count(),
            1,
            "shared should appear exactly once"
        );
    }

    #[test]
    fn enabled_worldbook_names_both_empty() {
        let session = Session::default();
        let cfg = Config::default();

        let names = enabled_worldbook_names(&session, &cfg);
        assert!(names.is_empty());
    }
}

pub(super) fn load_active_persona(app: &mut App) {
    if let Some(ref name) = app.session.persona {
        if let Some(ref db) = app.db {
            if let Ok(pf) = db.load_persona(name) {
                app.active_persona_name = Some(pf.name);
                app.active_persona_desc = Some(pf.persona);
                return;
            }
        }
    }
    app.active_persona_name = None;
    app.active_persona_desc = None;
}

pub fn new_chat_entry() -> SessionEntry {
    SessionEntry {
        id: String::new(),
        display_name: "+ New Chat".to_owned(),
        message_count: None,
        last_assistant_preview: None,
        sidebar_label: "+ New Chat".to_owned(),
        sidebar_preview: None,
        is_new_chat: true,
    }
}

fn truncate_preview(msg: &str) -> String {
    let sanitized: String = msg.chars().filter(|c| !c.is_control() || *c == ' ').collect();
    let truncated: String = sanitized.chars().take(SIDEBAR_PREVIEW_CHARS).collect();
    if sanitized.chars().count() > SIDEBAR_PREVIEW_CHARS {
        format!("  {truncated}...")
    } else {
        format!("  {truncated}")
    }
}

pub(crate) fn prepare_sidebar_entries(entries: &mut [SessionEntry]) {
    let mut name_totals: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for entry in entries.iter().filter(|entry| !entry.is_new_chat) {
        *name_totals.entry(entry.display_name.clone()).or_insert(0) += 1;
    }

    let mut name_remaining = name_totals;
    for entry in entries.iter_mut() {
        if entry.is_new_chat {
            entry.sidebar_label.clone_from(&entry.display_name);
            entry.sidebar_preview = None;
            continue;
        }

        let Some(rem) = name_remaining.get_mut(&entry.display_name) else {
            continue;
        };
        let idx = *rem;
        *rem -= 1;
        let count_str = entry
            .message_count
            .map(|n| format!(" ({n})"))
            .unwrap_or_default();
        entry.sidebar_label = format!("[{idx}] {}{count_str}", entry.display_name);
        entry.sidebar_preview = entry
            .last_assistant_preview
            .as_deref()
            .map(truncate_preview);
    }
}

pub(super) fn refresh_sidebar(app: &mut App) {
    let mut sessions = discover_sidebar_sessions(&app.save_mode, app.db.as_ref());

    let current_id = app.save_mode.id().map(str::to_owned);

    if let Some(ref cid) = current_id {
        if let Some(current_entry) = sessions.iter_mut().find(|e| e.id == *cid) {
            if let Some(ref character) = app.session.character {
                current_entry.display_name.clone_from(character);
            }
            current_entry.message_count = Some(app.session.tree.node_count());
            if current_entry.last_assistant_preview.is_none() {
                current_entry.last_assistant_preview = app
                    .session
                    .tree
                    .current_last_assistant_preview()
                    .map(str::to_owned);
            }
        }
    }

    let selected = current_id
        .and_then(|cid| sessions.iter().position(|s| s.id == cid))
        .unwrap_or(0);
    prepare_sidebar_entries(&mut sessions);
    app.sidebar_sessions = sessions;
    app.sidebar_state.select(Some(selected));
    app.sidebar_cache = None;
}

/// Loads the sidebar session list from the database, prepending a "New Chat" entry.
pub fn discover_sidebar_sessions(save_mode: &SaveMode, db: Option<&Database>) -> Vec<SessionEntry> {
    let mode = match save_mode {
        SaveMode::Database { .. } => "database",
        SaveMode::None => "none",
        SaveMode::PendingPasskey { .. } => "pending_passkey",
    };
    let mut sessions = libllm::debug_log::timed_kv(
        "startup.phase",
        &[
            libllm::debug_log::field("phase", "sidebar_population"),
            libllm::debug_log::field("mode", mode),
        ],
        || match save_mode {
            SaveMode::Database { .. } => {
                let Some(db) = db else { return Vec::new() };
                match db.list_sessions() {
                    Ok(entries) => entries
                        .into_iter()
                        .map(|e| SessionEntry {
                            id: e.id,
                            display_name: e.display_name,
                            message_count: Some(e.message_count),
                            last_assistant_preview: e.last_assistant_preview,
                            sidebar_label: String::new(),
                            sidebar_preview: None,
                            is_new_chat: false,
                        })
                        .collect(),
                    Err(e) => {
                        eprintln!("Warning: {e}");
                        Vec::new()
                    }
                }
            }
            SaveMode::None | SaveMode::PendingPasskey { .. } => Vec::new(),
        },
    );
    sessions.insert(0, new_chat_entry());
    prepare_sidebar_entries(&mut sessions);
    sessions
}
