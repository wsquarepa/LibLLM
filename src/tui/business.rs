use crate::session::{self, Message, Role, SaveMode, Session, SessionEntry};

use super::App;

pub fn non_empty(s: &str) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s.to_owned())
    }
}

pub fn build_effective_system_prompt(session: &Session) -> Option<String> {
    let cfg = crate::config::load();
    let base = session.system_prompt.as_deref().unwrap_or("");
    let is_character = session.character.is_some();
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
        result = result
            .replace("{{char}}", char_name)
            .replace("{{user}}", user_name);
    }

    Some(result)
}

pub fn inject_worldbook_entries<'a>(
    session: &Session,
    messages: &[&'a Message],
) -> Vec<Message> {
    if session.character.is_none() || session.worldbooks.is_empty() {
        return messages.iter().map(|m| (*m).clone()).collect();
    }

    let cfg = crate::config::load();
    let char_name = session.character.as_deref().unwrap_or("");
    let user_name = cfg.user_name.as_deref().unwrap_or("User");

    let msg_texts: Vec<&str> = messages.iter().map(|m| m.content.as_str()).collect();
    let wi_dir = crate::config::worldinfo_dir();

    let mut all_activated: Vec<crate::worldinfo::ActivatedEntry> = Vec::new();
    for wb_name in &session.worldbooks {
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

    for entry in all_activated.into_iter().rev() {
        let content = entry
            .content
            .replace("{{char}}", char_name)
            .replace("{{user}}", user_name);
        let insert_pos = if entry.depth == 0 || entry.depth >= len {
            0
        } else {
            len - entry.depth
        };
        result.insert(insert_pos, Message::new(Role::System, content));
    }

    result
}

pub fn replace_template_vars(session: &Session, messages: Vec<Message>) -> Vec<Message> {
    if session.character.is_none() {
        return messages;
    }

    let cfg = crate::config::load();
    let char_name = session.character.as_deref().unwrap_or("");
    let user_name = cfg.user_name.as_deref().unwrap_or("User");

    messages
        .into_iter()
        .map(|mut msg| {
            msg.content = msg
                .content
                .replace("{{char}}", char_name)
                .replace("{{user}}", user_name);
            msg
        })
        .collect()
}

pub fn load_config_fields() -> Vec<String> {
    let cfg = crate::config::load();
    let defaults = crate::sampling::SamplingParams::default();
    vec![
        cfg.api_url
            .unwrap_or_else(|| crate::config::Config::default().api_url().to_owned()),
        cfg.template.unwrap_or_else(|| "llama2".to_owned()),
        cfg.system_prompt.unwrap_or_default(),
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

pub fn save_config_from_fields(fields: &[String]) {
    let existing = crate::config::load();
    let cfg = crate::config::Config {
        api_url: non_empty(&fields[0]),
        template: non_empty(&fields[1]),
        system_prompt: non_empty(&fields[2]),
        user_name: existing.user_name,
        user_persona: existing.user_persona,
        sampling: crate::sampling::SamplingOverrides {
            temperature: fields[3].parse().ok(),
            top_k: fields[4].parse().ok(),
            top_p: fields[5].parse().ok(),
            min_p: fields[6].parse().ok(),
            repeat_last_n: fields[7].parse().ok(),
            repeat_penalty: fields[8].parse().ok(),
            max_tokens: fields[9].parse().ok(),
        },
    };

    let path = crate::config::config_path();
    if let Ok(toml_str) = toml::to_string_pretty(&cfg) {
        let _ = std::fs::write(path, toml_str);
    }
}

pub fn apply_config(app: &mut App) {
    let cfg = crate::config::load();
    let template_name = cfg.template.as_deref().unwrap_or("llama2");
    app.template = crate::prompt::Template::from_name(template_name);
    app.stop_tokens = app.template.stop_tokens();
    app.sampling =
        crate::sampling::SamplingParams::default().with_overrides(&cfg.sampling);

    if let Some(sp) = cfg.system_prompt {
        app.session.system_prompt = Some(sp);
    }
}

pub fn load_self_fields() -> Vec<String> {
    let cfg = crate::config::load();
    vec![
        cfg.user_name.unwrap_or_default(),
        cfg.user_persona.unwrap_or_default(),
    ]
}

pub fn save_self_fields(fields: &[String]) {
    let mut cfg = crate::config::load();
    cfg.user_name = non_empty(&fields[0]);
    cfg.user_persona = non_empty(&fields[1]);

    let path = crate::config::config_path();
    if let Ok(toml_str) = toml::to_string_pretty(&cfg) {
        let _ = std::fs::write(path, toml_str);
    }
}

pub fn new_chat_entry() -> SessionEntry {
    SessionEntry {
        path: std::path::PathBuf::new(),
        preview: String::new(),
        filename: "+ New Chat".to_owned(),
        is_new_chat: true,
    }
}

pub fn refresh_sidebar(app: &mut App) {
    let sessions = discover_sidebar_sessions(&app.save_mode);
    let current_path = app.save_mode.path().map(|p| p.to_path_buf());
    let selected = current_path
        .and_then(|cp| sessions.iter().position(|s| s.path == cp))
        .unwrap_or(0);
    app.sidebar_sessions = sessions;
    app.sidebar_state.select(Some(selected));
}

pub fn discover_sidebar_sessions(save_mode: &SaveMode) -> Vec<SessionEntry> {
    let mut sessions = match save_mode {
        SaveMode::Encrypted { .. } => {
            session::list_session_paths(&crate::config::sessions_dir())
        }
        SaveMode::Plaintext(path) => {
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
                        preview: String::new(),
                        filename,
                        is_new_chat: false,
                    }
                })
                .collect();
            entries.sort_by(|a, b| b.filename.cmp(&a.filename));
            entries
        }
        SaveMode::None | SaveMode::PendingPasskey(_) => Vec::new(),
    };
    sessions.insert(0, new_chat_entry());
    sessions
}
