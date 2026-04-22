//! Business logic helpers for system prompt assembly, worldbook injection, and config management.

use tokio::sync::mpsc;

use libllm::client::ApiClient;
use libllm::db::Database;
use libllm::preset::InstructPreset;
use libllm::sampling::SamplingParams;
use libllm::session::{Message, Role, SaveMode, Session, SessionEntry};
use libllm::tokenizer::{TokenCountUpdate, TokenCounter};
use libllm::worldinfo::{ActivatedEntry, RuntimeWorldBook};

use super::{App, BackgroundEvent};

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
pub fn build_effective_system_prompt(session: &Session, db: Option<&Database>) -> Option<String> {
    let is_character = session.character.is_some();

    let session_prompt = session.system_prompt.as_deref().unwrap_or("");

    let builtin_name = if is_character {
        libllm::system_prompt::BUILTIN_ROLEPLAY
    } else {
        libllm::system_prompt::BUILTIN_ASSISTANT
    };
    let resolved_default = db.and_then(|db| db.load_prompt(builtin_name).ok().map(|p| p.content));
    let config_default = resolved_default.as_deref().unwrap_or("");

    let base = if session_prompt.is_empty() {
        config_default
    } else {
        session_prompt
    };

    let persona = session
        .persona
        .as_ref()
        .and_then(|name| db.and_then(|db| db.load_persona(name).ok()));

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

pub fn load_runtime_worldbooks(enabled: &[String], db: Option<&Database>) -> Vec<RuntimeWorldBook> {
    {
        let _span = tracing::info_span!(
            "worldbook.runtime",
            phase = "hydrate",
            enabled_count = enabled.len()
        )
        .entered();
        let Some(db) = db else { return Vec::new() };
        enabled
            .iter()
            .filter_map(|wb_name| {
                db.load_worldbook(wb_name)
                    .ok()
                    .map(|wb| RuntimeWorldBook::from_worldbook(&wb))
            })
            .collect()
    }
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
        cfg.auth.kind().to_string(),
        cfg.template_preset
            .as_deref()
            .unwrap_or("Default")
            .to_owned(),
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
        cfg.summarization.keep_last.to_string(),
        cfg.summarization.prompt.clone(),
    ];

    let files = vec![
        cfg.files.enabled.to_string(),
        cfg.files.per_file_bytes.to_string(),
        cfg.files.per_message_bytes.to_string(),
        match cfg.files.summarize_mode {
            libllm::config::FileSummarizeMode::Eager => "eager".to_owned(),
            libllm::config::FileSummarizeMode::Lazy => "lazy".to_owned(),
        },
        cfg.files.summary_prompt.clone(),
    ];

    vec![general, sampling, backup, summarization, files]
}

pub fn config_locked_fields_by_section(overrides: &crate::cli::CliOverrides) -> Vec<Vec<usize>> {
    let mut general: Vec<usize> = Vec::new();
    let mut sampling: Vec<usize> = Vec::new();
    if overrides.api_url.is_some() {
        general.push(0);
    }
    let auth_overridden = overrides.auth_type.is_some()
        || overrides.auth_basic_username.is_some()
        || overrides.auth_basic_password.is_some()
        || overrides.auth_bearer_token.is_some()
        || overrides.auth_header_name.is_some()
        || overrides.auth_header_value.is_some()
        || overrides.auth_query_name.is_some()
        || overrides.auth_query_value.is_some();
    if auth_overridden {
        general.push(1);
    }
    if overrides.template.is_some() {
        general.push(3);
    }
    if overrides.tls_skip_verify {
        general.push(5);
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
    vec![general, sampling, Vec::new(), Vec::new(), Vec::new()]
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
    let files_section = &sections[4];

    let api_url = if locked[0].contains(&0) {
        existing.api_url.clone()
    } else {
        non_empty(&general[0])
    };
    // general[1] is the Authentication display label — written by the auth sub-dialog, not here.
    let template_preset = non_empty(&general[2]);
    let instruct_preset = if locked[0].contains(&3) {
        existing.instruct_preset.clone()
    } else {
        non_empty(&general[3])
    };
    let reasoning_preset = non_empty(&general[4]).filter(|v| !v.eq_ignore_ascii_case("OFF"));
    let tls_skip_verify = if locked[0].contains(&5) {
        existing.tls_skip_verify
    } else {
        general[5] == "true"
    };

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
        context_size: parse_usize_clamped(
            &summarization[2],
            512,
            libllm::config::MAX_SUMMARIZATION_CONTEXT_SIZE,
        ),
        trigger_threshold: parse_usize_clamped(&summarization[3], 1, 100),
        keep_last: parse_usize_clamped(&summarization[4], 1, 100),
        prompt: summarization[5].clone(),
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
        default_persona: existing.default_persona,
        macros: existing.macros,
        theme: existing.theme,
        theme_colors: existing.theme_colors,
        backup: backup_cfg,
        summarization: summarization_cfg,
        auth: existing.auth,
        files: libllm::config::FilesConfig {
            enabled: files_section[0] == "true",
            per_file_bytes: parse_usize_clamped(&files_section[1], 0, usize::MAX),
            per_message_bytes: parse_usize_clamped(&files_section[2], 0, usize::MAX),
            summarize_mode: match files_section.get(3).map(String::as_str) {
                Some("lazy") => libllm::config::FileSummarizeMode::Lazy,
                _ => libllm::config::FileSummarizeMode::Eager,
            },
            summary_prompt: match files_section.get(4) {
                Some(s) => s.clone(),
                None => libllm::config::FilesConfig::default().summary_prompt,
            },
        },
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

#[derive(Clone, Debug, PartialEq)]
struct EffectiveConnectionConfig {
    api_url: String,
    tls_skip_verify: bool,
    auth: libllm::config::Auth,
}

impl EffectiveConnectionConfig {
    fn from_config(cfg: &libllm::config::Config, cli_overrides: &crate::cli::CliOverrides) -> Self {
        Self {
            api_url: cli_overrides
                .api_url
                .as_deref()
                .unwrap_or(cfg.api_url())
                .to_owned(),
            tls_skip_verify: cli_overrides.tls_skip_verify || cfg.tls_skip_verify,
            auth: libllm::config::resolve_auth(cfg, &cli_overrides.auth_overrides()),
        }
    }

    fn build_client(&self) -> ApiClient {
        ApiClient::new(&self.api_url, self.tls_skip_verify, self.auth.clone())
    }
}

struct RuntimeReloadState {
    config: libllm::config::Config,
    instruct_preset: InstructPreset,
    reasoning_preset: Option<libllm::preset::ReasoningPreset>,
    stop_tokens: Vec<String>,
    sampling: SamplingParams,
    summarization_enabled: bool,
    local_context_limit: usize,
    theme: super::theme::Theme,
    connection: EffectiveConnectionConfig,
}

fn load_runtime_reload_state(cli_overrides: &crate::cli::CliOverrides) -> RuntimeReloadState {
    let config = libllm::config::load();
    let preset_name = cli_overrides
        .template
        .as_deref()
        .or(config.instruct_preset.as_deref())
        .unwrap_or("Mistral V3-Tekken");
    let instruct_preset = libllm::preset::resolve_instruct_preset(preset_name);
    let reasoning_preset = config
        .reasoning_preset
        .as_deref()
        .and_then(libllm::preset::resolve_reasoning_preset);

    RuntimeReloadState {
        reasoning_preset,
        stop_tokens: instruct_preset.stop_tokens(),
        sampling: libllm::sampling::SamplingParams::default()
            .with_overrides(&config.sampling)
            .with_overrides(&cli_overrides.sampling),
        summarization_enabled: config.summarization.enabled && !cli_overrides.no_summarize,
        local_context_limit: config.summarization.context_size,
        theme: super::theme::resolve_theme(&config),
        connection: EffectiveConnectionConfig::from_config(&config, cli_overrides),
        config,
        instruct_preset,
    }
}

async fn emit_startup_probe_events(
    client: ApiClient,
    tokenizer_tx: mpsc::Sender<TokenCountUpdate>,
    bg_tx: mpsc::Sender<BackgroundEvent>,
) {
    let result = client.fetch_model_name().await;
    let models_ok = result.is_ok();
    let _ = bg_tx.send(BackgroundEvent::ModelFetched(result)).await;

    if !models_ok {
        return;
    }

    let token_counter = TokenCounter::new(client.clone(), tokenizer_tx).await;
    let _ = bg_tx
        .send(BackgroundEvent::TokenizerReloaded(token_counter))
        .await;

    if let Some(server_ctx) = client.fetch_server_context_size().await {
        let _ = bg_tx
            .send(BackgroundEvent::ServerContextSize(server_ctx))
            .await;
    }
}

pub(super) fn spawn_startup_probes(
    client: ApiClient,
    tokenizer_tx: mpsc::Sender<TokenCountUpdate>,
    bg_tx: mpsc::Sender<BackgroundEvent>,
) {
    tokio::spawn(async move {
        emit_startup_probe_events(client, tokenizer_tx, bg_tx).await;
    });
}

pub(super) fn spawn_context_probe(client: ApiClient, bg_tx: mpsc::Sender<BackgroundEvent>) {
    tokio::spawn(async move {
        if let Some(server_ctx) = client.fetch_server_context_size().await {
            let _ = bg_tx
                .send(BackgroundEvent::ServerContextSize(server_ctx))
                .await;
        }
    });
}

fn build_summarize_client(
    config: &libllm::config::Config,
    cli_overrides: &crate::cli::CliOverrides,
) -> ApiClient {
    let auth = libllm::config::resolve_auth(config, &cli_overrides.auth_overrides());
    let url = summarize_api_url(config, cli_overrides);
    ApiClient::new(&url, config.tls_skip_verify || cli_overrides.tls_skip_verify, auth)
}

pub(super) fn summarize_api_url(
    config: &libllm::config::Config,
    cli_overrides: &crate::cli::CliOverrides,
) -> String {
    cli_overrides
        .api_url
        .clone()
        .or_else(|| config.summarization.api_url.clone())
        .unwrap_or_else(|| config.api_url().to_owned())
}

pub fn build_file_summarizer(
    db_path: &std::path::Path,
    key: Option<&std::sync::Arc<libllm::crypto::DerivedKey>>,
    config: &libllm::config::Config,
    cli_overrides: &crate::cli::CliOverrides,
    ready_tx: tokio::sync::mpsc::UnboundedSender<libllm::files::ReadyEvent>,
) -> anyhow::Result<std::sync::Arc<libllm::files::FileSummarizer>> {
    use anyhow::Context as _;
    let conn = {
        let _span = tracing::info_span!(
            "tui.file_summarizer.open_conn",
            path = %db_path.display()
        )
        .entered();
        rusqlite::Connection::open(db_path).context("open summarizer DB connection")?
    };
    if let Some(key) = key {
        conn.execute_batch(&key.key_pragma())
            .context("apply summarizer DB key")?;
    }
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
        .context("configure summarizer DB pragmas")?;
    let conn_arc = std::sync::Arc::new(std::sync::Mutex::new(conn));
    let summarize_client = build_summarize_client(config, cli_overrides);
    let api_url = summarize_api_url(config, cli_overrides);
    tracing::info!(
        api_url = %api_url,
        "tui.file_summarizer.construct.done"
    );
    Ok(std::sync::Arc::new(libllm::files::FileSummarizer::new(
        conn_arc,
        summarize_client,
        config.files.summary_prompt.clone(),
        ready_tx,
    )))
}

pub(super) fn apply_config(app: &mut App) {
    let previous_connection =
        EffectiveConnectionConfig::from_config(&app.config, &app.cli_overrides);
    let runtime = load_runtime_reload_state(&app.cli_overrides);

    app.file_summarizer = app.file_summarizer.as_ref().map(|existing| {
        std::sync::Arc::new(libllm::files::FileSummarizer::new(
            existing.conn_clone_for_reload(),
            build_summarize_client(&runtime.config, &app.cli_overrides),
            runtime.config.files.summary_prompt.clone(),
            existing.ready_tx_clone_for_reload(),
        ))
    });

    app.instruct_preset = runtime.instruct_preset;
    app.reasoning_preset = runtime.reasoning_preset;
    app.stop_tokens = runtime.stop_tokens;
    app.sampling = runtime.sampling;
    app.summarization_enabled = runtime.summarization_enabled;
    app.context_mgr.set_token_limit(runtime.local_context_limit);
    app.theme = runtime.theme;
    app.config = runtime.config;
    app.invalidate_worldbook_cache();
    app.invalidate_chat_cache();

    if runtime.connection != previous_connection {
        app.client = runtime.connection.build_client();
        app.model_name = None;
        app.api_available = true;
        app.api_error.clear();
        spawn_startup_probes(
            app.client.clone(),
            app.tokenizer_tx.clone(),
            app.bg_tx.clone(),
        );
    } else {
        spawn_context_probe(app.client.clone(), app.bg_tx.clone());
    }
}

pub fn build_theme_color_overrides(
    sections: &[Vec<String>],
) -> libllm::config::ThemeColorOverrides {
    let mut overrides = libllm::config::ThemeColorOverrides::default();
    for (tab_offset, labels) in crate::tui::dialogs::THEME_COLOR_TAB_LAYOUT
        .iter()
        .enumerate()
    {
        let section_idx = tab_offset + 1;
        for (field_idx, label) in labels.iter().enumerate() {
            overrides.set(*label, non_empty(&sections[section_idx][field_idx]));
        }
    }
    overrides
}

pub fn apply_theme_color_sections(
    sections: &[Vec<String>],
    existing: libllm::config::Config,
) -> anyhow::Result<()> {
    let base_theme = sections[0][0].clone();
    let overrides = build_theme_color_overrides(sections);

    let cfg = libllm::config::Config {
        theme: Some(base_theme),
        theme_colors: if overrides.any_set() {
            Some(overrides)
        } else {
            None
        },
        ..existing
    };

    libllm::config::save(&cfg)
}

pub(super) fn load_active_persona(app: &mut App) {
    if let Some(ref name) = app.session.persona
        && let Some(ref db) = app.db
        && let Ok(pf) = db.load_persona(name)
    {
        app.active_persona_name = Some(pf.name);
        app.active_persona_desc = Some(pf.persona);
        return;
    }
    app.active_persona_name = None;
    app.active_persona_desc = None;
}

pub fn new_chat_entry() -> SessionEntry {
    SessionEntry {
        id: String::new(),
        display_name: "+ New Chat".to_owned(),
        message_count: None,
        updated_at: None,
        sidebar_label: "+ New Chat".to_owned(),
        sidebar_preview: None,
        is_new_chat: true,
    }
}

/// Formats an RFC 3339 timestamp as an age relative to `now`: `1d`, `2h`, `5m`, or `now`.
/// Returns `None` when the timestamp cannot be parsed.
pub(crate) fn format_relative_age(timestamp: &str, now: time::OffsetDateTime) -> Option<String> {
    let parsed =
        time::OffsetDateTime::parse(timestamp, &time::format_description::well_known::Rfc3339)
            .ok()?;
    let diff = now - parsed;
    Some(if diff.whole_days() > 0 {
        format!("{}d", diff.whole_days())
    } else if diff.whole_hours() > 0 {
        format!("{}h", diff.whole_hours())
    } else if diff.whole_minutes() > 0 {
        format!("{}m", diff.whole_minutes())
    } else {
        "now".to_owned()
    })
}

pub(crate) fn prepare_sidebar_entries(entries: &mut [SessionEntry]) {
    let now = time::OffsetDateTime::now_utc();
    for entry in entries.iter_mut() {
        if entry.is_new_chat {
            entry.sidebar_label.clone_from(&entry.display_name);
            entry.sidebar_preview = None;
            continue;
        }

        let age = entry
            .updated_at
            .as_deref()
            .and_then(|ts| format_relative_age(ts, now))
            .unwrap_or_else(|| {
                tracing::warn!(
                    entry_id = %entry.id,
                    updated_at = ?entry.updated_at,
                    "sidebar: unparseable updated_at timestamp"
                );
                "?".to_owned()
            });
        let count = entry.message_count.unwrap_or(0);
        entry.sidebar_label = format!("{age} • {}", entry.display_name);
        entry.sidebar_preview = Some(format!("  {count} messages"));
    }
}

pub(super) fn refresh_sidebar(app: &mut App) {
    let mut sessions = discover_sidebar_sessions(&app.save_mode, app.db.as_ref());

    let current_id = app.save_mode.id().map(str::to_owned);

    if let Some(ref cid) = current_id
        && let Some(current_entry) = sessions.iter_mut().find(|e| e.id == *cid)
    {
        if let Some(ref character) = app.session.character {
            current_entry.display_name.clone_from(character);
        }
        current_entry.message_count = Some(app.session.tree.node_count());
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
    let mut sessions = {
        let _span =
            tracing::info_span!("startup.phase", phase = "sidebar_population", mode).entered();
        match save_mode {
            SaveMode::Database { .. } => {
                let Some(db) = db else { return Vec::new() };
                match db.list_sessions() {
                    Ok(entries) => entries
                        .into_iter()
                        .map(|e| SessionEntry {
                            id: e.id,
                            display_name: e.display_name,
                            message_count: Some(e.message_count),
                            updated_at: Some(e.updated_at),
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
        }
    };
    sessions.insert(0, new_chat_entry());
    prepare_sidebar_entries(&mut sessions);
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cli::CliOverrides;
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
        let session = Session {
            worldbooks: vec!["lore_a".to_owned(), "lore_b".to_owned()],
            ..Session::default()
        };
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
        let session = Session {
            worldbooks: vec!["shared".to_owned(), "session_only".to_owned()],
            ..Session::default()
        };
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

    #[test]
    fn runtime_reload_state_applies_summarization_and_connection_overrides() {
        let dir = tempfile::TempDir::new().unwrap();
        libllm::config::set_data_dir(dir.path().to_path_buf()).ok();

        let cfg = libllm::config::Config {
            api_url: Some("http://config.example/v1".to_owned()),
            tls_skip_verify: false,
            summarization: libllm::config::SummarizationConfig {
                enabled: true,
                api_url: None,
                context_size: 4096,
                trigger_threshold: 10,
                keep_last: 4,
                prompt: "summarize".to_owned(),
            },
            ..libllm::config::Config::default()
        };
        libllm::config::save(&cfg).unwrap();

        let cli_overrides = CliOverrides {
            api_url: Some("http://override.example/v1".to_owned()),
            template: None,
            tls_skip_verify: true,
            sampling: libllm::sampling::SamplingOverrides::default(),
            system_prompt: None,
            persona: None,
            no_summarize: true,
            auth_type: None,
            auth_basic_username: None,
            auth_basic_password: None,
            auth_bearer_token: None,
            auth_header_name: None,
            auth_header_value: None,
            auth_query_name: None,
            auth_query_value: None,
        };

        let runtime = load_runtime_reload_state(&cli_overrides);

        assert_eq!(runtime.connection.api_url, "http://override.example/v1");
        assert!(runtime.connection.tls_skip_verify);
        assert_eq!(runtime.local_context_limit, 4096);
        assert!(!runtime.summarization_enabled);
    }

    #[test]
    fn effective_connection_config_resolves_auth_overrides() {
        let cfg = Config {
            api_url: Some("http://config.example/v1".to_owned()),
            auth: libllm::config::Auth::Bearer {
                token: "config-token".to_owned(),
            },
            ..Config::default()
        };
        let cli_overrides = CliOverrides {
            api_url: None,
            template: None,
            tls_skip_verify: false,
            sampling: libllm::sampling::SamplingOverrides::default(),
            system_prompt: None,
            persona: None,
            no_summarize: false,
            auth_type: Some(libllm::config::AuthKind::Header),
            auth_basic_username: None,
            auth_basic_password: None,
            auth_bearer_token: None,
            auth_header_name: Some("X-Test-Auth".to_owned()),
            auth_header_value: Some("override-value".to_owned()),
            auth_query_name: None,
            auth_query_value: None,
        };

        let connection = EffectiveConnectionConfig::from_config(&cfg, &cli_overrides);

        assert_eq!(connection.api_url, "http://config.example/v1");
        assert_eq!(
            connection.auth,
            libllm::config::Auth::Header {
                name: "X-Test-Auth".to_owned(),
                value: "override-value".to_owned(),
            }
        );
    }

    #[test]
    fn config_editor_round_trips_files_summarize_fields() {
        let dir = tempfile::TempDir::new().unwrap();
        libllm::config::set_data_dir(dir.path().to_path_buf()).ok();

        let mut cfg = libllm::config::Config::default();
        cfg.files.summarize_mode = libllm::config::FileSummarizeMode::Lazy;
        cfg.files.summary_prompt = "Custom file summary prompt.".to_owned();

        let no_overrides = CliOverrides {
            api_url: None,
            template: None,
            tls_skip_verify: false,
            sampling: libllm::sampling::SamplingOverrides::default(),
            system_prompt: None,
            persona: None,
            no_summarize: false,
            auth_type: None,
            auth_basic_username: None,
            auth_basic_password: None,
            auth_bearer_token: None,
            auth_header_name: None,
            auth_header_value: None,
            auth_query_name: None,
            auth_query_value: None,
        };

        let sections = load_tabbed_config_sections(&cfg, &no_overrides);
        apply_tabbed_config_fields(&sections, cfg, &no_overrides).unwrap();

        let rebuilt = libllm::config::load();

        assert_eq!(
            rebuilt.files.summarize_mode,
            libllm::config::FileSummarizeMode::Lazy
        );
        assert_eq!(rebuilt.files.summary_prompt, "Custom file summary prompt.");
    }

    use time::macros::datetime;

    #[test]
    fn format_relative_age_days_hours_minutes_now() {
        let now = datetime!(2026-04-22 12:00:00 UTC);
        assert_eq!(
            format_relative_age("2026-04-20T12:00:00Z", now).as_deref(),
            Some("2d")
        );
        assert_eq!(
            format_relative_age("2026-04-22T09:30:00Z", now).as_deref(),
            Some("2h")
        );
        assert_eq!(
            format_relative_age("2026-04-22T11:45:00Z", now).as_deref(),
            Some("15m")
        );
        assert_eq!(
            format_relative_age("2026-04-22T11:59:30Z", now).as_deref(),
            Some("now")
        );
    }

    #[test]
    fn format_relative_age_unparseable_returns_none() {
        let now = datetime!(2026-04-22 12:00:00 UTC);
        assert!(format_relative_age("not-a-date", now).is_none());
        assert!(format_relative_age("", now).is_none());
    }

    fn make_session_entry(display_name: &str, updated_at: Option<&str>, count: Option<usize>) -> SessionEntry {
        SessionEntry {
            id: "id".to_owned(),
            display_name: display_name.to_owned(),
            message_count: count,
            updated_at: updated_at.map(str::to_owned),
            sidebar_label: String::new(),
            sidebar_preview: None,
            is_new_chat: false,
        }
    }

    #[test]
    fn prepare_sidebar_entries_preserves_new_chat_label() {
        let mut entries = vec![new_chat_entry()];
        prepare_sidebar_entries(&mut entries);
        assert_eq!(entries[0].sidebar_label, "+ New Chat");
        assert!(entries[0].sidebar_preview.is_none());
    }

    #[test]
    fn prepare_sidebar_entries_formats_label_and_preview() {
        let mut entries = vec![make_session_entry(
            "Alice",
            Some("2020-01-01T00:00:00Z"),
            Some(7),
        )];
        prepare_sidebar_entries(&mut entries);
        assert!(entries[0].sidebar_label.ends_with(" • Alice"));
        assert_eq!(entries[0].sidebar_preview.as_deref(), Some("  7 messages"));
    }

    #[test]
    fn prepare_sidebar_entries_marks_unparseable_timestamp() {
        let mut entries = vec![make_session_entry("Bob", Some("garbage"), Some(0))];
        prepare_sidebar_entries(&mut entries);
        assert_eq!(entries[0].sidebar_label, "? • Bob");
        assert_eq!(entries[0].sidebar_preview.as_deref(), Some("  0 messages"));
    }
}
