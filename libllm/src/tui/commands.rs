use anyhow::Result;
use tokio::sync::mpsc;

use libllm_core::client::StreamToken;
use libllm_core::session::{self, Message, Role, SaveMode};

use super::business::{self, load_config_fields, refresh_sidebar};
use super::{App, Focus, dialogs};

fn post_passkey_focus(app: &App) -> Focus {
    if app.model_name.is_none() {
        Focus::LoadingDialog
    } else if !app.api_available {
        Focus::ApiErrorDialog
    } else {
        Focus::Input
    }
}

fn loaded_worldbooks(app: &mut App) -> Vec<libllm_core::worldinfo::RuntimeWorldBook> {
    let enabled_names = super::business::enabled_worldbook_names(app.session, &app.config);
    let cache_stale = app
        .worldbook_cache
        .as_ref()
        .is_none_or(|cache| cache.enabled_names != enabled_names);

    if cache_stale {
        let books = libllm_core::debug_log::timed_kv(
            "worldbook.runtime",
            &[
                libllm_core::debug_log::field("phase", "load"),
                libllm_core::debug_log::field("cache", "miss"),
                libllm_core::debug_log::field("enabled_count", enabled_names.len()),
            ],
            || super::business::load_runtime_worldbooks(&enabled_names, app.db.as_ref()),
        );
        app.worldbook_cache = Some(super::WorldbookCache {
            enabled_names,
            books,
        });
    } else if let Some(cache) = app.worldbook_cache.as_ref() {
        libllm_core::debug_log::log_kv(
            "worldbook.runtime",
            &[
                libllm_core::debug_log::field("phase", "load"),
                libllm_core::debug_log::field("cache", "hit"),
                libllm_core::debug_log::field("enabled_count", enabled_names.len()),
                libllm_core::debug_log::field("book_count", cache.books.len()),
            ],
        );
    }

    app.worldbook_cache.as_ref().unwrap().books.clone()
}

pub(super) fn handle_slash_command(
    cmd: &str,
    arg: &str,
    app: &mut App,
    sender: mpsc::Sender<StreamToken>,
) {
    let cmd = libllm_core::commands::resolve_alias(cmd);
    match cmd {
        "/quit" => cmd_quit(app),
        "/clear" => cmd_clear(app),
        "/retry" => cmd_retry(app, sender),
        "/continue" => cmd_continue(app, sender),
        "/system" => cmd_system(app),
        "/config" => cmd_config(app),
        "/branch" => cmd_branch(app),
        "/persona" => cmd_persona(app),
        "/worldbook" => cmd_worldbook(app),
        "/character" => cmd_character(app),
        "/passkey" => cmd_passkey(app),
        "/theme" => cmd_theme(app, arg),
        "/export" => cmd_export(app, arg),
        "/macro" => cmd_macro(app, arg, sender),
        "/report" => cmd_report(app),
        _ => {
            app.set_status(
                format!("Unknown command: {cmd}"),
                super::StatusLevel::Warning,
            );
        }
    }
}

fn cmd_quit(app: &mut App) {
    app.should_quit = true;
}

fn cmd_clear(app: &mut App) {
    if !app.flush_session_before_transition() {
        return;
    }
    app.session.tree.clear();
    app.session.system_prompt = None;
    app.session.character = None;
    app.session.worldbooks.clear();
    app.session.persona = None;
    app.active_persona_name = None;
    app.active_persona_desc = None;
    app.discard_pending_session_save();
    app.invalidate_chat_cache();
    app.invalidate_worldbook_cache();
    app.chat_scroll = 0;
    app.auto_scroll = true;
    let new_id = session::generate_session_id();
    app.save_mode.set_id(new_id);
    refresh_sidebar(app);
}

fn cmd_retry(app: &mut App, sender: mpsc::Sender<StreamToken>) {
    app.nav_cursor = None;
    app.session.retreat_trailing_assistant();

    let last_user_content = app
        .session
        .tree
        .head()
        .and_then(|id| app.session.tree.node(id))
        .filter(|n| n.message.role == Role::User)
        .map(|n| n.message.content.clone());

    match last_user_content {
        Some(content) => {
            app.session.tree.retreat_head();
            start_streaming(app, &content, sender);
        }
        None => {
            app.set_status(
                "No user message to retry.".to_owned(),
                super::StatusLevel::Warning,
            );
        }
    }
}

fn cmd_continue(app: &mut App, sender: mpsc::Sender<StreamToken>) {
    app.nav_cursor = None;

    let head_is_assistant = app
        .session
        .tree
        .head()
        .and_then(|id| app.session.tree.node(id))
        .is_some_and(|n| n.message.role == Role::Assistant);

    if !head_is_assistant {
        app.set_status(
            "Cannot continue: last message is not from assistant.".to_owned(),
            super::StatusLevel::Warning,
        );
        return;
    }

    start_continuation(app, sender);
}

fn start_continuation(app: &mut App, sender: mpsc::Sender<StreamToken>) {
    if app.model_name.is_none() {
        app.set_status(
            "Connecting to API server...".to_owned(),
            super::StatusLevel::Warning,
        );
        return;
    }
    if !app.api_available {
        app.set_status(
            "Cannot send: API server is not available".to_owned(),
            super::StatusLevel::Error,
        );
        return;
    }

    app.is_streaming = true;
    app.is_continuation = true;
    app.streaming_buffer.clear();
    app.auto_scroll = true;

    let worldbooks = loaded_worldbooks(app);
    let branch_path = app.session.tree.branch_path();
    let truncated = app.context_mgr.truncated_path(&branch_path);
    let effective_prompt =
        super::business::build_effective_system_prompt(app.session, app.db.as_ref());
    let user_name = app.active_persona_name.as_deref().unwrap_or("User");
    let injected = super::business::inject_loaded_worldbook_entries(
        app.session,
        truncated,
        user_name,
        &worldbooks,
    );
    let injected = super::business::replace_template_vars(app.session, injected, user_name);
    let injected_refs: Vec<&Message> = injected.iter().collect();
    let prompt = app
        .instruct_preset
        .render_continuation(&injected_refs, effective_prompt.as_deref());
    let stop_tokens = app.stop_tokens.clone();
    let sampling = app.sampling.clone();

    let client = app.client.clone();
    let handle = tokio::spawn(async move {
        let stop_refs: Vec<&str> = stop_tokens.iter().map(String::as_str).collect();
        client
            .stream_completion_to_channel(&prompt, &stop_refs, &sampling, sender)
            .await;
    });
    app.streaming_task = Some(handle);
}

fn cmd_system(app: &mut App) {
    if app.cli_overrides.system_prompt.is_some() {
        let content = app
            .session
            .system_prompt
            .as_deref()
            .unwrap_or("")
            .to_owned();
        let values = vec!["(set via -r)".to_owned(), content];
        let dialog = dialogs::open_system_prompt_editor(values).with_locked_fields(vec![0, 1]);
        app.system_prompt_editor = Some(dialog);
        app.system_editor_read_only = true;
        app.system_editor_prompt_name = String::new();
        app.system_editor_return_focus = Focus::Input;
        app.focus = Focus::SystemPromptEditorDialog;
        return;
    }

    let prompts = app.db.as_ref().and_then(|db| db.list_prompts().ok()).unwrap_or_default();
    if prompts.is_empty() {
        app.set_status(
            "No system prompts found.".to_owned(),
            super::StatusLevel::Warning,
        );
    } else {
        app.system_prompt_list = prompts.into_iter().map(|(_, name, _)| name).collect();
        app.system_prompt_selected = 0;
        app.focus = Focus::SystemPromptDialog;
    }
}

fn cmd_config(app: &mut App) {
    let locked = business::config_locked_fields(&app.cli_overrides);
    app.config_dialog = Some(dialogs::open_config_editor(
        load_config_fields(&libllm_core::config::load(), &app.cli_overrides),
        locked,
    ));
    app.focus = Focus::ConfigDialog;
}

fn cmd_branch(app: &mut App) {
    let target = {
        let path_ids = app.session.tree.current_branch_ids();
        app.nav_cursor.or_else(|| {
            if path_ids.len() >= 2 {
                Some(path_ids[path_ids.len() - 2])
            } else {
                path_ids.last().copied()
            }
        })
    };

    let Some(target_id) = target else {
        app.set_status(
            "No messages to branch.".to_owned(),
            super::StatusLevel::Warning,
        );
        return;
    };

    let siblings = app.session.tree.siblings_of(target_id);
    if siblings.len() <= 1 {
        app.set_status(
            "No branches at this point.".to_owned(),
            super::StatusLevel::Warning,
        );
        return;
    }

    const BRANCH_PREVIEW_CHARS: usize = 60;
    app.branch_dialog_items = siblings
        .iter()
        .map(|&sib_id| {
            let node = app.session.tree.node(sib_id).unwrap();
            let content = &node.message.content;
            let preview = if content.len() > BRANCH_PREVIEW_CHARS {
                let end = content[..BRANCH_PREVIEW_CHARS]
                    .char_indices()
                    .last()
                    .map_or(0, |(i, c)| i + c.len_utf8());
                format!("{}...", &content[..end])
            } else {
                content.clone()
            };
            let preview = preview.replace('\n', " ");
            let label = format!("[{}] {}", node.message.role, preview);
            (sib_id, label)
        })
        .collect();

    let current_idx = siblings.iter().position(|&s| s == target_id).unwrap_or(0);
    app.branch_dialog_selected = current_idx;
    app.focus = Focus::BranchDialog;
}

fn cmd_persona(app: &mut App) {
    if let Some(ref persona_name) = app.cli_overrides.persona {
        let pf = app.db.as_ref().and_then(|db| db.load_persona(persona_name).ok());
        let values = match pf {
            Some(pf) => vec![pf.name, pf.persona],
            None => vec![persona_name.clone(), String::new()],
        };
        let all_locked = vec![0, 1];
        app.persona_editor_file_name = persona_name.clone();
        app.persona_editor =
            Some(dialogs::open_persona_editor(values).with_locked_fields(all_locked));
        app.focus = Focus::PersonaEditorDialog;
        return;
    }

    let personas = app.db.as_ref().and_then(|db| db.list_personas().ok()).unwrap_or_default();
    app.persona_list = personas.into_iter().map(|(_, name)| name).collect();
    app.persona_selected = 0;
    app.focus = Focus::PersonaDialog;
}

fn cmd_worldbook(app: &mut App) {
    let books = app.db.as_ref().and_then(|db| db.list_worldbooks().ok()).unwrap_or_default();
    app.worldbook_list = books.into_iter().map(|(_, name)| name).collect();
    app.worldbook_selected = 0;
    app.focus = Focus::WorldbookDialog;
}

fn cmd_character(app: &mut App) {
    let chars = app.db.as_ref().and_then(|db| db.list_characters().ok()).unwrap_or_default();
    app.character_names = chars.iter().map(|(_, name)| name.clone()).collect();
    app.character_slugs = chars.into_iter().map(|(slug, _)| slug).collect();
    app.character_selected = 0;
    app.focus = Focus::CharacterDialog;
}

fn cmd_passkey(app: &mut App) {
    match &app.save_mode {
        SaveMode::Database { .. } => {
            if app.db.is_some() {
                app.set_passkey_input.clear();
                app.set_passkey_confirm.clear();
                app.set_passkey_active_field = 0;
                app.set_passkey_error.clear();
                app.set_passkey_deriving = false;
                app.set_passkey_is_initial = false;
                app.focus = Focus::SetPasskeyDialog;
            } else {
                app.set_status(
                    "Database not available.".to_owned(),
                    super::StatusLevel::Error,
                );
            }
        }
        SaveMode::None => {
            app.set_status(
                "Encryption is disabled for this session.".to_owned(),
                super::StatusLevel::Warning,
            );
        }
        SaveMode::PendingPasskey { .. } => {
            app.set_status(
                "Please unlock sessions first.".to_owned(),
                super::StatusLevel::Warning,
            );
        }
    }
}

fn cmd_theme(app: &mut App, arg: &str) {
    let arg = arg.trim();
    if arg.is_empty() {
        let current = app.config.theme.as_deref().unwrap_or("dark");
        let available = super::theme::Theme::available_themes().join(", ");
        app.set_status(
            format!("Current theme: {current}. Available: {available}"),
            super::StatusLevel::Info,
        );
        return;
    }

    if super::theme::Theme::from_name(arg).is_none() {
        let available = super::theme::Theme::available_themes().join(", ");
        app.set_status(
            format!("Unknown theme: {arg}. Available: {available}"),
            super::StatusLevel::Error,
        );
        return;
    }

    app.config.theme = Some(arg.to_owned());
    app.theme = super::theme::resolve_theme(&app.config);
    app.invalidate_chat_cache();

    if let Err(err) = libllm_core::config::save(&app.config) {
        app.set_status(
            format!("Theme applied but failed to save config: {err}"),
            super::StatusLevel::Warning,
        );
    } else {
        app.set_status(
            format!("Switched to {arg} theme"),
            super::StatusLevel::Info,
        );
    }
}

enum ExportFormat {
    Markdown,
    Html,
    Jsonl,
}

impl ExportFormat {
    fn parse(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "" | "html" => Ok(Self::Html),
            "md" | "markdown" => Ok(Self::Markdown),
            "jsonl" | "json" => Ok(Self::Jsonl),
            other => Err(format!("Unknown export format: {other}. Use md, html, or jsonl")),
        }
    }

    fn extension(&self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Html => "html",
            Self::Jsonl => "jsonl",
        }
    }
}

fn cmd_export(app: &mut App, arg: &str) {
    let format = match ExportFormat::parse(arg.trim()) {
        Ok(f) => f,
        Err(err) => {
            app.set_status(err, super::StatusLevel::Error);
            return;
        }
    };

    let messages = app.session.tree.branch_path();
    if messages.is_empty() {
        app.set_status(
            "Nothing to export (empty conversation)".to_owned(),
            super::StatusLevel::Warning,
        );
        return;
    }

    let current_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            app.set_status(
                format!("Cannot resolve current directory: {err}"),
                super::StatusLevel::Error,
            );
            return;
        }
    };

    let char_name = app.session.character.as_deref().unwrap_or("Assistant");
    let user_name = app
        .active_persona_name
        .as_deref()
        .unwrap_or("User");

    let content = match format {
        ExportFormat::Markdown => libllm_core::export::render_markdown(&messages, char_name, user_name),
        ExportFormat::Html => libllm_core::export::render_html(&messages, char_name, user_name),
        ExportFormat::Jsonl => libllm_core::export::render_jsonl(&messages, char_name, user_name),
    };

    let timestamp = session::now_compact();
    let filename = format!("export-{timestamp}.{}", format.extension());
    let output_path = current_dir.join(&filename);

    match std::fs::write(&output_path, content) {
        Ok(()) => app.set_status(
            format!("Exported to {}", output_path.display()),
            super::StatusLevel::Info,
        ),
        Err(err) => app.set_status(
            format!("Failed to write export: {err}"),
            super::StatusLevel::Error,
        ),
    }
}

fn cmd_macro(app: &mut App, arg: &str, sender: mpsc::Sender<StreamToken>) {
    let arg = arg.trim();
    if arg.is_empty() {
        let names: Vec<&String> = app.config.macros.keys().collect();
        if names.is_empty() {
            app.set_status(
                "No macros defined. Add [macros] to config.toml".to_owned(),
                super::StatusLevel::Warning,
            );
        } else {
            let list = names
                .iter()
                .map(|n| n.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            app.set_status(
                format!("Available macros: {list}"),
                super::StatusLevel::Info,
            );
        }
        return;
    }

    let (name, macro_args) = match arg.split_once(' ') {
        Some((n, rest)) => (n, rest),
        None => (arg, ""),
    };

    let template = match app.config.macros.get(name) {
        Some(t) => t.clone(),
        None => {
            app.set_status(
                format!("Unknown macro: {name}"),
                super::StatusLevel::Warning,
            );
            return;
        }
    };

    match expand_macro(&template, macro_args) {
        Ok(expanded) => start_streaming(app, &expanded, sender),
        Err(err) => app.set_status(err, super::StatusLevel::Error),
    }
}

#[derive(Debug, PartialEq)]
enum Placeholder {
    All,
    Single(usize),
    Range(usize, usize),
    Greedy(usize),
}

fn parse_placeholder(content: &str) -> Result<Placeholder, String> {
    let content = content.trim();
    if content.is_empty() {
        return Ok(Placeholder::All);
    }

    if let Some(rest) = content.strip_suffix("...") {
        if rest.is_empty() {
            return Err("Invalid placeholder: {{...}}".to_owned());
        }
        let n: usize = rest
            .parse()
            .map_err(|_| format!("Invalid placeholder: {{{{{content}}}}}"))?;
        if n == 0 {
            return Err("Placeholder indices start at 1".to_owned());
        }
        return Ok(Placeholder::Greedy(n));
    }

    if let Some(rest) = content.strip_suffix("..") {
        if rest.is_empty() {
            return Err("Invalid placeholder: {{..}}".to_owned());
        }
        let n: usize = rest
            .parse()
            .map_err(|_| format!("Invalid placeholder: {{{{{content}}}}}"))?;
        if n == 0 {
            return Err("Placeholder indices start at 1".to_owned());
        }
        return Ok(Placeholder::Greedy(n));
    }

    if let Some(dot_pos) = content.find("...") {
        let left = &content[..dot_pos];
        let right = &content[dot_pos + 3..];
        let a: usize = left
            .parse()
            .map_err(|_| format!("Invalid placeholder: {{{{{content}}}}}"))?;
        let b: usize = right
            .parse()
            .map_err(|_| format!("Invalid placeholder: {{{{{content}}}}}"))?;
        if a == 0 || b == 0 {
            return Err("Placeholder indices start at 1".to_owned());
        }
        if a > b {
            return Err(format!("Invalid range: {a}...{b} (start > end)"));
        }
        return Ok(Placeholder::Range(a, b));
    }

    if let Some(dot_pos) = content.find("..") {
        let left = &content[..dot_pos];
        let right = &content[dot_pos + 2..];
        let a: usize = left
            .parse()
            .map_err(|_| format!("Invalid placeholder: {{{{{content}}}}}"))?;
        let b: usize = right
            .parse()
            .map_err(|_| format!("Invalid placeholder: {{{{{content}}}}}"))?;
        if a == 0 || b == 0 {
            return Err("Placeholder indices start at 1".to_owned());
        }
        if a > b {
            return Err(format!("Invalid range: {a}..{b} (start > end)"));
        }
        return Ok(Placeholder::Range(a, b));
    }

    let n: usize = content
        .parse()
        .map_err(|_| format!("Invalid placeholder: {{{{{content}}}}}"))?;
    if n == 0 {
        return Err("Placeholder indices start at 1".to_owned());
    }
    Ok(Placeholder::Single(n))
}

enum ScanItem {
    Escaped(usize, usize),
    Placeholder(usize, usize, Placeholder),
}

fn scan_template(template: &str) -> Result<Vec<ScanItem>, String> {
    let mut result = Vec::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 2 < bytes.len() && bytes[i + 1] == b'{' && bytes[i + 2] == b'{' {
            result.push(ScanItem::Escaped(i, i + 1));
            i += 1;
            continue;
        }
        if bytes[i] == b'{' && i > 0 && bytes[i - 1] == b'\\' {
            i += 1;
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let start = i;
            let inner_start = i + 2;
            let mut j = inner_start;
            let mut found = false;
            while j + 1 < bytes.len() {
                if bytes[j] == b'}' && bytes[j + 1] == b'}' {
                    let content = &template[inner_start..j];
                    let placeholder = parse_placeholder(content)?;
                    result.push(ScanItem::Placeholder(start, j + 2, placeholder));
                    i = j + 2;
                    found = true;
                    break;
                }
                j += 1;
            }
            if !found {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    Ok(result)
}

fn validate_placeholders(items: &[ScanItem]) -> Result<(), String> {
    let mut covered_ranges: Vec<(usize, usize)> = Vec::new();
    let mut singles: Vec<usize> = Vec::new();
    let mut has_all = false;

    for item in items {
        if let ScanItem::Placeholder(_, _, ph) = item {
            match ph {
                Placeholder::All => has_all = true,
                Placeholder::Single(n) => singles.push(*n),
                Placeholder::Range(a, b) => covered_ranges.push((*a, *b)),
                Placeholder::Greedy(a) => covered_ranges.push((*a, usize::MAX)),
            }
        }
    }

    if has_all {
        return Ok(());
    }

    for &n in &singles {
        for &(start, end) in &covered_ranges {
            if n >= start && n <= end {
                return Err(format!(
                    "Placeholder {{{{{n}}}}} overlaps with range {{{{{start}..{end}}}}}"
                ));
            }
        }
    }

    if singles.is_empty() && covered_ranges.is_empty() {
        return Ok(());
    }

    let mut max_idx: usize = 0;
    for &n in &singles {
        max_idx = max_idx.max(n);
    }
    for &(start, end) in &covered_ranges {
        max_idx = max_idx.max(start);
        if end != usize::MAX {
            max_idx = max_idx.max(end);
        }
    }

    for idx in 1..=max_idx {
        let in_single = singles.contains(&idx);
        let in_range = covered_ranges.iter().any(|&(s, e)| idx >= s && idx <= e);
        if !in_single && !in_range {
            return Err(format!(
                "Gap at index {idx}: all indices from 1 to {max_idx} must be covered"
            ));
        }
    }

    Ok(())
}

pub fn expand_macro(template: &str, raw_args: &str) -> Result<String, String> {
    let items = scan_template(template)?;
    validate_placeholders(&items)?;

    let args: Vec<&str> = if raw_args.trim().is_empty() {
        Vec::new()
    } else {
        raw_args.split_whitespace().collect()
    };

    let mut result = String::with_capacity(template.len());
    let mut last_end = 0;

    for item in &items {
        match item {
            ScanItem::Escaped(start, skip_to) => {
                result.push_str(&template[last_end..*start]);
                last_end = *skip_to;
            }
            ScanItem::Placeholder(start, end, ph) => {
                result.push_str(&template[last_end..*start]);
                match ph {
                    Placeholder::All => result.push_str(raw_args),
                    Placeholder::Single(n) => {
                        if let Some(arg) = args.get(*n - 1) {
                            result.push_str(arg);
                        }
                    }
                    Placeholder::Range(a, b) => {
                        let from = (*a - 1).min(args.len());
                        let to = (*b).min(args.len());
                        let slice = &args[from..to];
                        result.push_str(&slice.join(" "));
                    }
                    Placeholder::Greedy(a) => {
                        let from = (*a - 1).min(args.len());
                        let slice = &args[from..];
                        result.push_str(&slice.join(" "));
                    }
                }
                last_end = *end;
            }
        }
    }

    result.push_str(&template[last_end..]);
    Ok(result)
}

fn cmd_report(app: &mut App) {
    if !libllm_core::config::load().debug_log {
        app.set_status(
            "Debug logging is disabled in config".to_owned(),
            super::StatusLevel::Error,
        );
        return;
    }
    let current_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            app.set_status(
                format!("Cannot resolve current directory: {err}"),
                super::StatusLevel::Error,
            );
            return;
        }
    };
    let output_path = current_dir.join("debug.log");
    if output_path.exists() {
        app.set_status(
            format!("Refusing to overwrite existing {}", output_path.display()),
            super::StatusLevel::Error,
        );
        return;
    }

    match libllm_core::debug_log::copy_current_log_to(&output_path) {
        Ok(()) => app.set_status(
            format!("Debug log copied to {}", output_path.display()),
            super::StatusLevel::Info,
        ),
        Err(err) => app.set_status(
            format!("Failed to write debug report: {err}"),
            super::StatusLevel::Error,
        ),
    }
}

pub(super) fn start_streaming(app: &mut App, content: &str, sender: mpsc::Sender<StreamToken>) {
    if app.model_name.is_none() {
        app.set_status(
            "Connecting to API server...".to_owned(),
            super::StatusLevel::Warning,
        );
        return;
    }
    if !app.api_available {
        app.set_status(
            "Cannot send: API server is not available".to_owned(),
            super::StatusLevel::Error,
        );
        return;
    }
    let parent = app.session.tree.head();
    app.session
        .tree
        .push(parent, Message::new(Role::User, content.to_owned()));
    app.mark_session_dirty(super::SaveTrigger::Debounced, false);
    app.invalidate_chat_cache();
    app.is_streaming = true;
    app.focus = super::Focus::Input;
    app.nav_cursor = None;
    app.hover_node = None;
    app.streaming_buffer.clear();
    app.auto_scroll = true;

    let worldbooks = loaded_worldbooks(app);
    let branch_path = app.session.tree.branch_path();
    let truncated = app.context_mgr.truncated_path(&branch_path);
    let effective_prompt =
        super::business::build_effective_system_prompt(app.session, app.db.as_ref());
    let user_name = app.active_persona_name.as_deref().unwrap_or("User");
    let injected = super::business::inject_loaded_worldbook_entries(
        app.session,
        truncated,
        user_name,
        &worldbooks,
    );
    let injected = super::business::replace_template_vars(app.session, injected, user_name);
    let injected_refs: Vec<&Message> = injected.iter().collect();
    let prompt = app
        .instruct_preset
        .render(&injected_refs, effective_prompt.as_deref());
    let stop_tokens = app.stop_tokens.clone();
    let sampling = app.sampling.clone();

    let client = app.client.clone();
    let handle = tokio::spawn(async move {
        let stop_refs: Vec<&str> = stop_tokens.iter().map(String::as_str).collect();
        client
            .stream_completion_to_channel(&prompt, &stop_refs, &sampling, sender)
            .await;
    });
    app.streaming_task = Some(handle);
}

pub(super) fn handle_stream_token(
    token: StreamToken,
    app: &mut App,
    sender: mpsc::Sender<StreamToken>,
) -> Result<()> {
    if !app.is_streaming {
        return Ok(());
    }
    match token {
        StreamToken::Token(text) => {
            app.streaming_buffer.push_str(&text);
            app.auto_scroll = true;
        }
        StreamToken::Done(full_response) => {
            let head = app.session.tree.head().unwrap();
            if app.is_continuation {
                let existing = app.session.tree.node(head).unwrap().message.content.clone();
                let combined = format!("{}{}", existing, full_response);
                app.session.tree.set_message_content(head, combined);
                app.is_continuation = false;
            } else {
                app.session
                    .tree
                    .push(Some(head), Message::new(Role::Assistant, full_response));
            }
            app.mark_session_dirty(super::SaveTrigger::StreamDone, true);
            app.invalidate_chat_cache();
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.auto_scroll = true;
            app.flush_session_save(super::SaveTrigger::StreamDone)?;
            refresh_sidebar(app);
            if !app.message_queue.is_empty() {
                let next = app.message_queue.remove(0);
                start_streaming(app, &next, sender);
                if !app.is_streaming {
                    app.message_queue.clear();
                }
            }
        }
        StreamToken::Error(err) => {
            app.streaming_buffer.clear();
            app.is_streaming = false;
            app.is_continuation = false;
            app.message_queue.clear();
            app.set_status(format!("Error: {err}"), super::StatusLevel::Error);
        }
    }
    Ok(())
}

pub(super) fn handle_background_event(event: super::BackgroundEvent, app: &mut App) {
    match event {
        super::BackgroundEvent::KeyDerived(key, db_path) => {
            if let Some(debug) = app.unlock_debug.take() {
                libllm_core::debug_log::log_kv(
                    "unlock.phase",
                    &[
                        libllm_core::debug_log::field("phase", "ui_complete"),
                        libllm_core::debug_log::field("kind", debug.kind),
                        libllm_core::debug_log::field("result", "ok"),
                        libllm_core::debug_log::field(
                            "elapsed_ms",
                            format!("{:.3}", debug.started_at.elapsed().as_secs_f64() * 1000.0),
                        ),
                    ],
                );
            }
            match libllm_core::db::Database::open(&db_path, Some(&key)) {
                Ok(db) => {
                    if let Err(e) = db.ensure_builtin_prompts() {
                        app.set_status(format!("Warning: {e}"), super::StatusLevel::Warning);
                    }
                    let id = match &app.save_mode {
                        SaveMode::PendingPasskey { id } => id.clone(),
                        _ => session::generate_session_id(),
                    };
                    app.db = Some(db);
                    app.save_mode = SaveMode::Database { id };
                    if let Err(err) = app.flush_session_save(super::SaveTrigger::Unlock) {
                        app.set_status(format!("Save error: {err}"), super::StatusLevel::Error);
                    }
                    app.invalidate_worldbook_cache();
                    app.passkey_deriving = false;
                    app.focus = post_passkey_focus(app);
                    refresh_sidebar(app);
                }
                Err(err) => {
                    app.passkey_deriving = false;
                    app.passkey_error = format!("Failed to open database: {err}");
                }
            }
        }
        super::BackgroundEvent::KeyDeriveFailed(err) => {
            if let Some(debug) = app.unlock_debug.take() {
                libllm_core::debug_log::log_kv(
                    "unlock.phase",
                    &[
                        libllm_core::debug_log::field("phase", "ui_complete"),
                        libllm_core::debug_log::field("kind", debug.kind),
                        libllm_core::debug_log::field("result", "error"),
                        libllm_core::debug_log::field(
                            "elapsed_ms",
                            format!("{:.3}", debug.started_at.elapsed().as_secs_f64() * 1000.0),
                        ),
                        libllm_core::debug_log::field("error", &err),
                    ],
                );
            }
            app.passkey_deriving = false;
            app.passkey_error = format!("Failed: {err}");
        }
        super::BackgroundEvent::PasskeySet(new_key) => {
            if let Some(debug) = app.unlock_debug.take() {
                libllm_core::debug_log::log_kv(
                    "unlock.phase",
                    &[
                        libllm_core::debug_log::field("phase", "ui_complete"),
                        libllm_core::debug_log::field("kind", debug.kind),
                        libllm_core::debug_log::field("result", "ok"),
                        libllm_core::debug_log::field(
                            "elapsed_ms",
                            format!("{:.3}", debug.started_at.elapsed().as_secs_f64() * 1000.0),
                        ),
                    ],
                );
            }
            app.set_passkey_deriving = false;
            app.invalidate_worldbook_cache();
            if app.set_passkey_is_initial {
                let db_path = libllm_core::config::data_dir().join("data.db");
                match libllm_core::db::Database::open(&db_path, Some(&new_key)) {
                    Ok(db) => {
                        if let Err(e) = db.ensure_builtin_prompts() {
                            app.set_status(format!("Warning: {e}"), super::StatusLevel::Warning);
                        }
                        let id = match &app.save_mode {
                            SaveMode::PendingPasskey { id } => id.clone(),
                            _ => session::generate_session_id(),
                        };
                        app.db = Some(db);
                        app.save_mode = SaveMode::Database { id };
                        if let Err(err) = app.flush_session_save(super::SaveTrigger::Unlock) {
                            app.set_status(format!("Save error: {err}"), super::StatusLevel::Error);
                        }
                        app.focus = post_passkey_focus(app);
                        refresh_sidebar(app);
                    }
                    Err(err) => {
                        app.set_status(format!("Failed to create database: {err}"), super::StatusLevel::Error);
                    }
                }
            } else {
                if let Some(ref db) = app.db {
                    match db.rekey(&new_key) {
                        Ok(()) => {
                            let check_path = libllm_core::config::key_check_path();
                            if let Err(err) = libllm_core::crypto::set_key_fingerprint(&check_path, &new_key) {
                                app.set_status(
                                    format!("Failed to update key fingerprint: {err}"),
                                    super::StatusLevel::Error,
                                );
                                return;
                            }
                            app.passkey_changed = true;
                            app.should_quit = true;
                        }
                        Err(err) => {
                            app.set_status(
                                format!("Failed to change passkey: {err}"),
                                super::StatusLevel::Error,
                            );
                        }
                    }
                } else {
                    app.set_status(
                        "No database available for rekey.".to_owned(),
                        super::StatusLevel::Error,
                    );
                }
                app.focus = Focus::Input;
            }
        }
        super::BackgroundEvent::PasskeySetFailed(err) => {
            if let Some(debug) = app.unlock_debug.take() {
                libllm_core::debug_log::log_kv(
                    "unlock.phase",
                    &[
                        libllm_core::debug_log::field("phase", "ui_complete"),
                        libllm_core::debug_log::field("kind", debug.kind),
                        libllm_core::debug_log::field("result", "error"),
                        libllm_core::debug_log::field(
                            "elapsed_ms",
                            format!("{:.3}", debug.started_at.elapsed().as_secs_f64() * 1000.0),
                        ),
                        libllm_core::debug_log::field("error", &err),
                    ],
                );
            }
            app.set_passkey_deriving = false;
            app.set_passkey_error = format!("Failed: {err}");
        }
        super::BackgroundEvent::ModelFetched(Ok(name)) => {
            app.model_name = Some(name);
            if app.focus == Focus::LoadingDialog {
                app.focus = Focus::Input;
            }
        }
        super::BackgroundEvent::ModelFetched(Err(err)) => {
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
    }
}
