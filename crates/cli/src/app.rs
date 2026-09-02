//! Application entry point and startup orchestration for the LibLLM client.

use libllm_core::character;
use libllm_core::crypto;
use libllm_core::preset;
use libllm_core::sampling;
use libllm_core::session;
use libllm_diagnostics as diagnostics;
use libllm_protocol::client::ApiClient;
use libllm_storage::db::Database;

use crate::cli;
use crate::edit;
use crate::import;
use crate::legacy_migration;
use crate::recover;
use crate::update;
use crate::validation;

use std::io::{self, IsTerminal, Read, Write};

use anyhow::{Context, Result};
#[cfg(debug_assertions)]
use chrono::Utc;
use clap::Parser;
use crossterm::execute;
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};

use cli::Args;
use session::{Message, Role, SaveMode};

pub async fn run() -> anyhow::Result<()> {
    libllm_protocol::crypto_provider::install_default_crypto_provider();
    let args = Args::parse();

    if args.version {
        println!("{}", crate::version::LONG);
        return Ok(());
    }

    if args.cleanup {
        let summary = diagnostics::cleanup_temp_logs()?;
        println!(
            "Removed {} temporary debug log(s); {} removal(s) failed.",
            summary.removed, summary.failed
        );
        return Ok(());
    }

    {
        const CHANNEL: &str = env!("LIBLLM_CHANNEL");
        if CHANNEL == "unknown" && args.data.is_none() {
            let default_data_dir = libllm_config::data_dir();
            execute!(
                io::stderr(),
                SetAttribute(Attribute::Bold),
                SetForegroundColor(Color::Red),
                Print("You are running a dev build. Use --data/-d to specify a data directory.\n"),
                ResetColor,
                SetAttribute(Attribute::Reset),
                SetForegroundColor(Color::DarkGrey),
                Print(format!(
                    "Run with \"libllm --data {}\" to bypass this warning.\n",
                    default_data_dir.display()
                )),
                ResetColor,
            )?;
            std::process::exit(1);
        }
    }

    if let Some(ref data_path) = args.data {
        let is_existing_dir = validation::validate_data_dir(data_path, args.no_encrypt)?;
        libllm_config::set_data_dir(data_path.clone())?;

        if is_existing_dir {
            let is_encrypted_dir = libllm_config::salt_path().exists();
            if is_encrypted_dir && args.no_encrypt {
                anyhow::bail!("Data directory is encrypted; --no-encrypt cannot be used with it.");
            }
            if !is_encrypted_dir && args.passkey.is_some() {
                anyhow::bail!("Data directory is not encrypted; --passkey cannot be used with it.");
            }
        }
    }

    #[cfg(debug_assertions)]
    if args.debug_trigger_destroy_all {
        return debug_trigger_destroy_all();
    }

    let cli_args_joined = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let build = diagnostics::BuildInfo {
        version: env!("CARGO_PKG_VERSION"),
        channel: env!("LIBLLM_CHANNEL"),
        commit: env!("LIBLLM_COMMIT"),
        dirty: !env!("LIBLLM_GIT_DIRTY").is_empty(),
    };
    let filter_env = std::env::var("LIBLLM_LOG").ok();
    let _diagnostics = diagnostics::init(diagnostics::InitParams {
        debug_override: args.debug.as_deref(),
        timings_path: args.timings.as_deref(),
        run_mode: infer_run_mode(&args),
        cli_args: cli_args_joined,
        build,
        filter_flag: args.log_filter.as_deref(),
        filter_env: filter_env.as_deref(),
    })?;

    libllm_core::timed_result!(
        tracing::Level::INFO,
        "startup.phase",
        phase = "ensure_dirs" ;
        { libllm_config::ensure_dirs() }
    )?;

    if let Some(cli::Command::Update { branch, yes }) = &args.command {
        return update::run(branch.clone(), *yes).await;
    }

    if let Some(cli::Command::Recover { command }) = &args.command {
        let data_dir = args
            .data
            .as_deref()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(libllm_config::data_dir);
        let passkey = resolve_recover_passkey(&args, &data_dir)?;
        return recover::run(&data_dir, passkey.as_deref(), command.as_ref());
    }

    if let Some(cli::Command::Db { command }) = &args.command {
        return cli::db::dispatch(&args, command);
    }

    if let Some(cli::Command::Search {
        query,
        limit,
        json,
        full,
    }) = &args.command
    {
        return cli::search::dispatch(&args, query, *limit, *json, *full);
    }

    libllm_config::migrate_config();

    legacy_migration::check_and_run_migration(args.no_encrypt, args.passkey.as_deref()).await?;

    if let Some(cli::Command::Import { files, kind }) = &args.command {
        let db = resolve_edit_db(&args)?;
        return import::handle_import_command(files, kind.as_deref(), &db);
    }

    if let Some(cli::Command::Edit { kind, name }) = &args.command {
        let db = resolve_edit_db(&args)?;
        return edit::handle_edit_command(kind, name, &db);
    }

    let cfg = {
        let _span = tracing::info_span!("startup.phase", phase = "config_load").entered();
        libllm_config::load()
    };

    let api_url = args.api_url.as_deref().unwrap_or_else(|| cfg.api_url());
    let tls_skip_verify = if args.tls_skip_verify {
        true
    } else {
        cfg.tls_skip_verify
    };
    if tls_skip_verify {
        crossterm::execute!(
            io::stderr(),
            SetForegroundColor(Color::Yellow),
            Print("Warning: TLS certificate verification is disabled.\n"),
            ResetColor,
        )?;
    }
    let cli_overrides = args.cli_overrides();
    let auth = libllm_core::config::resolve_auth(&cfg, &cli_overrides.auth_overrides());
    let client = ApiClient::new(api_url, tls_skip_verify, auth);

    let preset_name = args
        .template
        .as_deref()
        .or(cfg.instruct_preset.as_deref())
        .unwrap_or("Mistral V3-Tekken");
    let instruct_preset =
        preset::resolve_instruct_preset(preset_name, &libllm_config::instruct_presets_dir());
    let reasoning_preset = cfg
        .reasoning_preset
        .as_deref()
        .and_then(|n| preset::resolve_reasoning_preset(n, &libllm_config::reasoning_presets_dir()));
    let template_preset_name = cfg.template_preset.as_deref().unwrap_or("Default");
    let template_preset = preset::resolve_template_preset(
        template_preset_name,
        &libllm_config::template_presets_dir(),
    );

    let sampling = sampling::SamplingParams::default()
        .with_overrides(&cfg.sampling)
        .with_overrides(&args.sampling_overrides());

    let (mut session, mut save_mode, mut db, summarizer_db_path, summarizer_key) = libllm_core::timed_result!(
        tracing::Level::INFO,
        "startup.phase",
        phase = "resolve_session" ;
        { resolve_session(&args) }
    )?;

    session.template = Some(instruct_preset.name.clone());

    {
        if let Some(ref persona_name) = args.persona {
            session.persona = Some(character::slugify(persona_name));
        } else if session.persona.is_none() && session.tree.head().is_none() {
            session.persona = cfg.default_persona.clone();
        }

        if let Some(ref sp) = args.system_prompt {
            session.system_prompt = Some(sp.clone());
        } else if session.system_prompt.is_none()
            && let Some(ref db) = db
        {
            session.system_prompt = db
                .load_prompt(libllm_core::system_prompt::BUILTIN_ASSISTANT)
                .ok()
                .map(|p| p.content);
        }

        if let Some(ref text) = args.note {
            if text.trim().is_empty() {
                session.author_note = None;
            } else {
                session.author_note = Some(libllm_core::author_note::AuthorNote {
                    text: text.clone(),
                    depth: args
                        .note_depth
                        .unwrap_or(libllm_core::author_note::DEFAULT_DEPTH),
                    at_top: args.note_top,
                });
            }
        }

        if !args.character.is_empty() {
            let talkativeness_overrides =
                cli::parse_talkativeness(args.talkativeness.as_deref().unwrap_or(""))
                    .context("parsing --talkativeness")?;

            let mut card_names_by_slug = std::collections::HashMap::new();
            let mut loaded_cards: Vec<(String, character::CharacterCard)> = Vec::new();
            for slug in &args.character {
                let card = libllm_core::timed_result!(
                    tracing::Level::INFO,
                    "startup.phase",
                    phase = "resolve_character",
                    character = slug.as_str() ;
                    { resolve_character(slug, db.as_ref()) }
                )?;
                card_names_by_slug.insert(slug.clone(), card.name.clone());
                loaded_cards.push((slug.clone(), card));
            }

            if let Err(e) = cli::validate_group_chat_args(
                &args.character,
                &talkativeness_overrides,
                &card_names_by_slug,
            ) {
                eprintln!("error: {e}");
                std::process::exit(2);
            }

            if args.character.len() == 1 {
                let (_, card) = &loaded_cards[0];
                session.system_prompt =
                    Some(character::build_system_prompt(card, Some(&template_preset)));
                session.character = Some(card.name.clone());
                if session.tree.head().is_none() && !card.first_mes.is_empty() {
                    session
                        .tree
                        .push(None, Message::new(Role::Assistant, card.first_mes.clone()));
                }
            } else if session.tree.head().is_none() {
                for (slug, card) in &loaded_cards {
                    if card.first_mes.is_empty() {
                        continue;
                    }
                    let parent = session.tree.head();
                    let mut msg = Message::new(Role::Assistant, card.first_mes.clone());
                    msg.speaker = Some(slug.clone());
                    session.tree.push(parent, msg);
                }
            }

            let mut characters = Vec::new();
            for (slug, _) in &loaded_cards {
                let mut attachment =
                    libllm_core::group_chat::CharacterAttachment::new(slug.clone());
                if let Some(t) = talkativeness_overrides.get(slug.as_str()) {
                    attachment.talkativeness = *t;
                }
                characters.push(attachment);
            }
            session.characters = characters;
            session.chat_mode = args.chat_mode.into();
        }

        if let Some(ref text) = args.scenario {
            session.scenario = Some(text.clone());
        }
    }

    if !args.character.is_empty() {
        let new_id = session::generate_session_id();
        save_mode.set_id(new_id);
    }

    if let Some(ref message) = args.message {
        let (text, stdin_attachment) = if message == "-" {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            (buf, None)
        } else if !io::stdin().is_terminal() {
            let mut bytes = Vec::new();
            io::stdin().read_to_end(&mut bytes)?;
            let attachment = if bytes.is_empty() {
                None
            } else {
                match libllm_core::files::stdin_attachment(bytes, &cfg.files) {
                    Ok(rf) => Some(rf),
                    Err(err) => {
                        eprintln!("{err}");
                        std::process::exit(1);
                    }
                }
            };
            let text = if attachment.is_some() {
                format!("{message} @stdin")
            } else {
                message.clone()
            };
            (text, attachment)
        } else {
            (message.clone(), None)
        };

        let cwd = std::env::current_dir().context("read current working directory")?;
        let prepended = stdin_attachment.into_iter().collect::<Vec<_>>();
        let resolved_files = match libllm_core::files::resolve_with_prepended_resolved(
            prepended, &text, &cwd, &cfg.files,
        ) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        };

        if cfg.summarization.enabled && !resolved_files.is_empty() {
            let (refresh_tx, _refresh_rx) = tokio::sync::mpsc::channel(8);
            let token_counter =
                libllm_protocol::tokenizer::TokenCounter::new(client.clone(), refresh_tx).await;
            for file in &resolved_files {
                if let Err(err) = libllm_protocol::summarize::check_file_fits(
                    &token_counter,
                    file,
                    &cfg.files.summary_prompt,
                    cfg.summarization.context_size,
                )
                .await
                {
                    eprintln!("{err}");
                    std::process::exit(1);
                }
            }
        }

        let system_messages =
            match libllm_core::files::assemble_snapshot_messages(resolved_files, &cfg.files) {
                Ok(v) => v,
                Err(err) => {
                    eprintln!("{err}");
                    std::process::exit(1);
                }
            };

        let effective_prompt =
            libllm_tui::business::build_effective_system_prompt(&session, db.as_ref());

        let mut parent = session.tree.head();
        for sys_msg in system_messages {
            let id = session.tree.push(parent, sys_msg);
            parent = Some(id);
        }
        let user_node = session.tree.push(parent, Message::new(Role::User, text));

        let branch_path_msgs: Vec<Message> = session
            .tree
            .branch_path()
            .into_iter()
            .map(|m| match m.role {
                Role::User => Message {
                    role: m.role,
                    content: libllm_core::files::rewrite_user_message(&m.content),
                    timestamp: m.timestamp.clone(),
                    thought_seconds: m.thought_seconds,
                    speaker: m.speaker.clone(),
                    pre_turn_action_points: m.pre_turn_action_points.clone(),
                },
                _ => m.clone(),
            })
            .collect();
        let branch_refs: Vec<&Message> = branch_path_msgs.iter().collect();

        let prompt_text = reasoning_preset.as_ref().map_or_else(
            || instruct_preset.render(&branch_refs, effective_prompt.as_deref()),
            |preset| {
                preset.apply_prefix(
                    &instruct_preset.render(&branch_refs, effective_prompt.as_deref()),
                )
            },
        );
        let stop_tokens = instruct_preset.stop_tokens();
        let stop_refs: Vec<&str> = stop_tokens.iter().map(String::as_str).collect();
        let mut stdout = io::stdout().lock();
        let response = client
            .stream_completion(&prompt_text, &stop_refs, &sampling, &mut stdout)
            .await?;
        writeln!(stdout)?;

        session
            .tree
            .push(Some(user_node), Message::new(Role::Assistant, response));

        libllm_storage::db::save_session_for_mode(&save_mode, &session, db.as_mut())?;

        if let Some(id) = save_mode.id() {
            eprintln!("Session: {id}");
        }

        try_backup(
            &libllm_config::data_dir(),
            args.passkey.as_deref(),
            &cfg.backup,
        );

        return Ok(());
    }

    tracing::info!(phase = "tui_handoff", mode = "interactive", "startup.phase");
    let resolved_passkey = libllm_tui::run(
        client,
        &mut session,
        save_mode,
        db,
        instruct_preset,
        sampling,
        cli_overrides,
        libllm_tui::SummarizerParams {
            db_path: summarizer_db_path,
            derived_key: summarizer_key,
        },
        crate::version::STATUS_BAR,
    )
    .await?;

    let effective_passkey = resolved_passkey.as_deref().or(args.passkey.as_deref());
    let current_config = libllm_config::load();
    try_backup(
        &libllm_config::data_dir(),
        effective_passkey,
        &current_config.backup,
    );

    Ok(())
}

fn infer_run_mode(args: &Args) -> &'static str {
    if args.cleanup {
        "cleanup"
    } else if let Some(command) = &args.command {
        match command {
            cli::Command::Edit { .. } => "edit_subcommand",
            cli::Command::Import { .. } => "import_subcommand",
            cli::Command::Recover { .. } => "recover_subcommand",
            cli::Command::Update { .. } => "update_subcommand",
            cli::Command::Db { .. } => "db_subcommand",
            cli::Command::Search { .. } => "search_subcommand",
        }
    } else if args.message.is_some() {
        "single_message"
    } else {
        "tui"
    }
}

type ResolvedSession = (
    session::Session,
    SaveMode,
    Option<Database>,
    Option<std::path::PathBuf>,
    Option<std::sync::Arc<crypto::DerivedKey>>,
);

fn resolve_session(args: &Args) -> Result<ResolvedSession> {
    if args.message.is_some() && args.data.is_none() {
        return Ok((
            session::Session::default(),
            SaveMode::None,
            None,
            None,
            None,
        ));
    }

    let db_path = libllm_config::data_dir().join("data.db");

    if args.no_encrypt {
        let db = Database::open(&db_path, None)?;
        db.ensure_builtin_prompts()?;
        preset::ensure_default_presets(
            &libllm_config::instruct_presets_dir(),
            &libllm_config::reasoning_presets_dir(),
            &libllm_config::template_presets_dir(),
        );
        let id = session::generate_session_id();
        if let Some(ref uuid) = args.continue_session {
            let session = db.load_session(uuid)?;
            return Ok((
                session,
                SaveMode::Database { id: uuid.clone() },
                Some(db),
                Some(db_path),
                None,
            ));
        }
        return Ok((
            session::Session::default(),
            SaveMode::Database { id },
            Some(db),
            Some(db_path),
            None,
        ));
    }

    if let Some(ref passkey) = args.passkey {
        let salt = crypto::load_or_create_salt(&libllm_config::salt_path())?;
        let key = crypto::derive_key(passkey, &salt)?;
        let key_arc = std::sync::Arc::new(key);
        let db = Database::open(&db_path, Some(&*key_arc))
            .context("Wrong passkey (or corrupt database).")?;
        db.ensure_builtin_prompts()?;
        preset::ensure_default_presets(
            &libllm_config::instruct_presets_dir(),
            &libllm_config::reasoning_presets_dir(),
            &libllm_config::template_presets_dir(),
        );
        let id = session::generate_session_id();
        if let Some(ref uuid) = args.continue_session {
            let session = db.load_session(uuid)?;
            return Ok((
                session,
                SaveMode::Database { id: uuid.clone() },
                Some(db),
                Some(db_path),
                Some(key_arc),
            ));
        }
        return Ok((
            session::Session::default(),
            SaveMode::Database { id },
            Some(db),
            Some(db_path),
            Some(key_arc),
        ));
    }

    let id = session::generate_session_id();
    Ok((
        session::Session::default(),
        SaveMode::PendingPasskey { id },
        None,
        None,
        None,
    ))
}

fn resolve_character(char_arg: &str, db: Option<&Database>) -> Result<character::CharacterCard> {
    let path = std::path::Path::new(char_arg);
    if path.exists() {
        let card = character::import_card(path)?;
        if let Some(db) = db {
            let slug = character::slugify(&card.name);
            db.insert_character(&slug, &card)?;
        }
        return Ok(card);
    }

    let slug = character::slugify(char_arg);
    if let Some(db) = db
        && let Ok(card) = db.load_character(&slug)
    {
        return Ok(card);
    }

    anyhow::bail!("Character not found: {char_arg}");
}

/// Resolves the passkey to use for the `recover` subcommand.
///
/// Returns `None` for plaintext data directories (`--no-encrypt` or empty dir with no `.salt` and
/// no `data.db`), honours `--passkey` / `LIBLLM_PASSKEY` when present, and otherwise prompts on
/// the controlling terminal. Refuses to proceed when `data.db` exists but `.salt` is missing
/// unless `--no-encrypt` is explicit: that combination would silently restore plaintext backups
/// over a potentially-encrypted database. Fails with a clear message when the directory is
/// encrypted but no passkey can be obtained (non-interactive invocation without the flag/env var).
fn resolve_recover_passkey(args: &Args, data_dir: &std::path::Path) -> Result<Option<String>> {
    if args.no_encrypt {
        return Ok(None);
    }
    if let Some(pk) = &args.passkey {
        return Ok(Some(pk.clone()));
    }
    if !data_dir.join(".salt").exists() {
        if data_dir.join("data.db").exists() {
            anyhow::bail!(
                "data directory has data.db but no .salt: {}\n\
                 pass --no-encrypt to open it as plaintext, or restore the .salt file before proceeding",
                data_dir.display()
            );
        }
        return Ok(None);
    }
    if !crate::interactive::is_interactive() {
        anyhow::bail!(
            "data directory is encrypted but no passkey was provided; \
             pass --passkey or set LIBLLM_PASSKEY"
        );
    }
    eprint!("Passkey: ");
    let entered = rpassword::read_password().context("failed to read interactive passkey")?;
    if entered.is_empty() {
        anyhow::bail!("no passkey provided");
    }
    Ok(Some(entered))
}

fn resolve_edit_db(args: &Args) -> Result<Database> {
    let db_path = libllm_config::data_dir().join("data.db");

    if args.no_encrypt {
        return Ok(Database::open(&db_path, None)?);
    }

    let passkey: String = match args.passkey.clone() {
        Some(passkey) => passkey,
        None => {
            eprint!("Passkey: ");
            rpassword::read_password().context("failed to read interactive passkey")?
        }
    };

    let salt = crypto::load_or_create_salt(&libllm_config::salt_path())?;
    let key = crypto::derive_key(&passkey, &salt)?;
    Database::open(&db_path, Some(&key)).context("Wrong passkey (or corrupt database).")
}

fn try_backup(
    data_dir: &std::path::Path,
    passkey: Option<&str>,
    config: &libllm_core::config::BackupConfig,
) {
    if !config.enabled {
        return;
    }

    if !data_dir.join("data.db").exists() {
        return;
    }

    if passkey.is_none() && data_dir.join(".salt").exists() {
        return;
    }

    if let Err(err) = libllm_backup::snapshot::create_snapshot(data_dir, passkey, config) {
        eprintln!("Warning: backup failed: {err}");
    }
}

#[cfg(debug_assertions)]
fn debug_trigger_destroy_all() -> Result<()> {
    let data_dir = libllm_config::data_dir();
    let snapshot_path = std::env::temp_dir().join(format!(
        "libllm-{}.tar.zst",
        Utc::now().format("%Y%m%d-%H%M%S-debug")
    ));
    libllm_core::archive::snapshot_data_dir(&data_dir, &snapshot_path, "backups")
        .map_err(|e| anyhow::anyhow!("snapshot failed: {e}"))?;
    std::fs::remove_dir_all(&data_dir).map_err(|e| anyhow::anyhow!("delete failed: {e}"))?;
    eprintln!(
        "LibLLM data destroyed. Snapshot saved to: {}",
        snapshot_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_backup_skips_when_encrypted_and_no_passkey() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path();

        std::fs::write(data_dir.join("data.db"), b"not a real database").expect("write data.db");
        std::fs::write(data_dir.join(".salt"), b"salt-bytes").expect("write .salt");

        let config = libllm_core::config::BackupConfig::default();
        try_backup(data_dir, None, &config);

        assert!(
            !data_dir.join("backups").exists(),
            "try_backup must not touch an encrypted database without a passkey",
        );
    }
}
