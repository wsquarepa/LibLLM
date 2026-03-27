use crate::session::{self, Message, Role, SaveMode, Session, SessionEntry};

use super::App;

pub fn non_empty(s: &str) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s.to_owned())
    }
}

pub fn apply_template_vars(text: &str, char_name: &str, user_name: &str) -> String {
    text.replace("{{char}}", char_name).replace("{{user}}", user_name)
}

pub fn build_effective_system_prompt(
    session: &Session,
    cfg: &crate::config::Config,
) -> Option<String> {
    let is_character = session.character.is_some();

    let session_prompt = session.system_prompt.as_deref().unwrap_or("");
    let config_default = if is_character {
        cfg.roleplay_system_prompt.as_deref().unwrap_or("")
    } else {
        cfg.system_prompt.as_deref().unwrap_or("")
    };

    let base = if session_prompt.is_empty() {
        config_default
    } else {
        session_prompt
    };

    let has_persona = is_character && (cfg.user_name.is_some() || cfg.user_persona.is_some());

    if base.is_empty() && !has_persona {
        return None;
    }

    let mut parts: Vec<String> = Vec::new();
    if !base.is_empty() {
        parts.push(base.to_owned());
    }
    if has_persona {
        let name = cfg.user_name.as_deref().unwrap_or("the user");
        let mut persona_line = format!("The user's name is {name}.");
        if let Some(ref desc) = cfg.user_persona {
            if !desc.is_empty() {
                persona_line.push_str(&format!(" {desc}"));
            }
        }
        parts.push(persona_line);
    }

    let mut result = parts.join("\n\n");
    if is_character {
        let char_name = session.character.as_deref().unwrap_or("");
        let user_name = cfg.user_name.as_deref().unwrap_or("User");
        result = apply_template_vars(&result, char_name, user_name);
    }

    Some(result)
}

pub fn inject_worldbook_entries<'a>(
    session: &Session,
    messages: &[&'a Message],
    cfg: &crate::config::Config,
) -> Vec<Message> {
    if session.character.is_none() {
        return messages.iter().map(|m| (*m).clone()).collect();
    }

    let mut enabled: Vec<&String> = cfg.worldbooks.iter().collect();
    for name in &session.worldbooks {
        if !enabled.iter().any(|n| *n == name) {
            enabled.push(name);
        }
    }

    if enabled.is_empty() {
        return messages.iter().map(|m| (*m).clone()).collect();
    }

    let char_name = session.character.as_deref().unwrap_or("");
    let user_name = cfg.user_name.as_deref().unwrap_or("User");

    let msg_texts: Vec<&str> = messages.iter().map(|m| m.content.as_str()).collect();
    let wi_dir = crate::config::worldinfo_dir();

    let mut all_activated: Vec<crate::worldinfo::ActivatedEntry> = Vec::new();
    for wb_name in &enabled {
        let wb_path = wi_dir.join(format!("{wb_name}.json"));
        if let Ok(wb) = crate::worldinfo::load_worldbook(&wb_path) {
            all_activated.extend(crate::worldinfo::scan_entries(&wb, &msg_texts));
        }
    }

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
    cfg: &crate::config::Config,
) -> Vec<Message> {
    if session.character.is_none() {
        return messages;
    }

    let char_name = session.character.as_deref().unwrap_or("");
    let user_name = cfg.user_name.as_deref().unwrap_or("User");

    messages
        .into_iter()
        .map(|mut msg| {
            msg.content = apply_template_vars(&msg.content, char_name, user_name);
            msg
        })
        .collect()
}

pub fn load_config_fields(cfg: &crate::config::Config) -> Vec<String> {
    let defaults = crate::sampling::SamplingParams::default();
    vec![
        cfg.api_url.as_deref()
            .unwrap_or(crate::config::Config::default().api_url())
            .to_owned(),
        cfg.template.as_deref().unwrap_or("llama2").to_owned(),
        cfg.sampling
            .temperature
            .unwrap_or(defaults.temperature)
            .to_string(),
        cfg.sampling.top_k.unwrap_or(defaults.top_k).to_string(),
        cfg.sampling.top_p.unwrap_or(defaults.top_p).to_string(),
        cfg.sampling.min_p.unwrap_or(defaults.min_p).to_string(),
        cfg.sampling
            .repeat_last_n
            .unwrap_or(defaults.repeat_last_n)
            .to_string(),
        cfg.sampling
            .repeat_penalty
            .unwrap_or(defaults.repeat_penalty)
            .to_string(),
        cfg.sampling
            .max_tokens
            .unwrap_or(defaults.max_tokens)
            .to_string(),
    ]
}

pub fn save_config_from_fields(fields: &[String]) -> anyhow::Result<()> {
    let existing = crate::config::load();
    let cfg = crate::config::Config {
        api_url: non_empty(&fields[0]),
        template: non_empty(&fields[1]),
        system_prompt: existing.system_prompt,
        roleplay_system_prompt: existing.roleplay_system_prompt,
        user_name: existing.user_name,
        user_persona: existing.user_persona,
        worldbooks: existing.worldbooks,
        sampling: crate::sampling::SamplingOverrides {
            temperature: fields[2].parse().ok(),
            top_k: fields[3].parse().ok(),
            top_p: fields[4].parse().ok(),
            min_p: fields[5].parse().ok(),
            repeat_last_n: fields[6].parse().ok(),
            repeat_penalty: fields[7].parse().ok(),
            max_tokens: fields[8].parse().ok(),
        },
    };

    crate::config::save(&cfg)
}

pub fn apply_config(app: &mut App) {
    let cfg = crate::config::load();
    let template_name = cfg.template.as_deref().unwrap_or("llama2");
    app.template = crate::prompt::Template::from_name(template_name);
    app.stop_tokens = app.template.stop_tokens();
    app.sampling =
        crate::sampling::SamplingParams::default().with_overrides(&cfg.sampling);

    let is_character = app.session.character.is_some();
    let prompt = if is_character {
        cfg.roleplay_system_prompt.clone()
    } else {
        cfg.system_prompt.clone()
    };
    if let Some(sp) = prompt {
        app.session.system_prompt = Some(sp);
    }

    app.user_name = cfg.user_name.clone();
    app.config = cfg;
}

pub fn load_self_fields(cfg: &crate::config::Config) -> Vec<String> {
    vec![
        cfg.user_name.clone().unwrap_or_default(),
        cfg.user_persona.clone().unwrap_or_default(),
    ]
}

pub fn save_self_fields(fields: &[String]) -> anyhow::Result<()> {
    let mut cfg = crate::config::load();
    cfg.user_name = non_empty(&fields[0]);
    cfg.user_persona = non_empty(&fields[1]);

    crate::config::save(&cfg)
}

pub fn new_chat_entry() -> SessionEntry {
    SessionEntry {
        path: std::path::PathBuf::new(),
        filename: "+ New Chat".to_owned(),
        display_name: "+ New Chat".to_owned(),
        message_count: None,
        first_message: None,
        is_new_chat: true,
    }
}

pub fn refresh_sidebar(app: &mut App) {
    let mut sessions = discover_sidebar_sessions(&app.save_mode);

    for entry in &mut sessions {
        if entry.is_new_chat {
            continue;
        }
        if let Some(cached) = app.sidebar_sessions.iter().find(|e| e.path == entry.path) {
            if cached.display_name != "Assistant" {
                entry.display_name.clone_from(&cached.display_name);
            }
            if cached.message_count.is_some() {
                entry.message_count = cached.message_count;
            }
            if cached.first_message.is_some() {
                entry.first_message.clone_from(&cached.first_message);
            }
        }
    }

    let current_path = app.save_mode.path().map(|p| p.to_path_buf());

    if let Some(ref cp) = current_path {
        if let Some(current_entry) = sessions.iter_mut().find(|e| e.path == *cp) {
            if let Some(ref character) = app.session.character {
                current_entry.display_name.clone_from(character);
            }
            current_entry.message_count = Some(app.session.tree.node_count());
            if current_entry.first_message.is_none() {
                current_entry.first_message = app
                    .session
                    .tree
                    .branch_path()
                    .into_iter()
                    .find(|m| m.role == crate::session::Role::User)
                    .map(|m| m.content.clone());
            }
        }
    }

    let selected = current_path
        .and_then(|cp| sessions.iter().position(|s| s.path == cp))
        .unwrap_or(0);
    app.sidebar_sessions = sessions;
    app.sidebar_state.select(Some(selected));

    if let crate::session::SaveMode::Encrypted { key, .. } = &app.save_mode {
        let bg_tx = app.bg_tx.clone();
        super::commands::spawn_metadata_loading(&app.sidebar_sessions, key, &bg_tx);
    }
}

pub fn discover_sidebar_sessions(save_mode: &SaveMode) -> Vec<SessionEntry> {
    let mut sessions = match save_mode {
        SaveMode::Encrypted { .. } => {
            match session::list_session_paths(&crate::config::sessions_dir()) {
                Ok(sessions) => sessions,
                Err(e) => {
                    eprintln!("Warning: {e}");
                    Vec::new()
                }
            }
        }
        SaveMode::Plaintext(path) => list_plaintext_sessions(path),
        SaveMode::None | SaveMode::PendingPasskey(_) => Vec::new(),
    };
    sessions.insert(0, new_chat_entry());
    sessions
}

fn list_plaintext_sessions(path: &std::path::Path) -> Vec<SessionEntry> {
    let dir = match path.parent() {
        Some(d) => d,
        None => return Vec::new(),
    };
    let mut entries: Vec<SessionEntry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .map(|p| {
            let filename = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            SessionEntry {
                path: p,
                filename,
                display_name: "Assistant".to_owned(),
                message_count: None,
                first_message: None,
                is_new_chat: false,
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        let mtime = |p: &std::path::Path| {
            p.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        };
        mtime(&b.path).cmp(&mtime(&a.path))
    });
    entries
}
