mod character;
mod cli;
mod client;
mod commands;
mod config;
mod context;
mod crypto;
mod debug_log;
mod index;
mod migration;
mod persona;
mod preset;
mod sampling;
mod session;
mod system_prompt;
mod tui;
mod update;
mod worldinfo;

use std::io::{self, Read, Write};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;

use cli::Args;
use client::ApiClient;
use session::{Message, Role, SaveMode};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    if args.cleanup {
        let summary = debug_log::cleanup_temp_logs()?;
        println!(
            "Removed {} temporary debug log(s); {} removal(s) failed.",
            summary.removed, summary.failed
        );
        return Ok(());
    }

    {
        const CHANNEL: &str = env!("LIBLLM_CHANNEL");
        if !matches!(CHANNEL, "stable" | "nightly") && args.data.is_none() {
            use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
            use crossterm::execute;

            let default_data_dir = config::data_dir();
            let _ = execute!(
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
            );
            std::process::exit(1);
        }
    }

    if args.no_encrypt && args.data.is_none() {
        anyhow::bail!("--no-encrypt requires --data/-d to specify a data directory.");
    }
    if args.passkey.is_some() && args.data.is_none() {
        anyhow::bail!("--passkey requires --data/-d to specify a data directory.");
    }
    if args.continue_session.is_some() && args.data.is_none() {
        anyhow::bail!("--continue requires --data/-d to specify a data directory.");
    }
    if args.continue_session.is_some() && args.message.is_none() {
        anyhow::bail!("--continue can only be used with -m.");
    }

    if let Some(ref data_path) = args.data {
        let is_existing_dir = if data_path.exists() {
            if !data_path.is_dir() {
                anyhow::bail!("--data path exists but is not a directory: {}", data_path.display());
            }
            let is_empty = std::fs::read_dir(data_path)
                .with_context(|| format!("failed to read --data directory: {}", data_path.display()))?
                .next()
                .is_none();
            if !is_empty {
                let has_config = data_path.join("config.toml").exists()
                    || data_path.join("sessions").exists();
                if !has_config {
                    anyhow::bail!(
                        "--data directory is not empty and does not appear to be a libllm data directory: {}",
                        data_path.display()
                    );
                }
            }
            !is_empty
        } else {
            std::fs::create_dir_all(data_path)
                .with_context(|| format!("failed to create --data directory: {}", data_path.display()))?;
            false
        };
        config::set_data_dir(data_path.clone());

        if is_existing_dir {
            let is_encrypted_dir = config::key_check_path().exists();
            if is_encrypted_dir && args.no_encrypt {
                anyhow::bail!(
                    "Data directory is encrypted; --no-encrypt cannot be used with it."
                );
            }
            if !is_encrypted_dir && args.passkey.is_some() {
                anyhow::bail!(
                    "Data directory is not encrypted; --passkey cannot be used with it."
                );
            }
        }
    }

    let debug_enabled = args.debug.is_some() || config::load().debug_log;
    let _diagnostics = if debug_enabled {
        Some(debug_log::init(
            args.debug.as_deref(),
            args.timings.as_deref(),
            infer_run_mode(&args),
            &build_run_fields(&args),
        )?)
    } else {
        None
    };

    crate::debug_log::timed_result(
        "startup.phase",
        &[crate::debug_log::field("phase", "ensure_dirs")],
        config::ensure_dirs,
    )?;

    migration::migrate_config_path();

    if let Some(cli::Command::Edit { kind, name }) = &args.command {
        return handle_edit_command(kind, name, &args);
    }

    if let Some(cli::Command::Update { nightly }) = &args.command {
        return update::run(*nightly).await;
    }

    let cfg = crate::debug_log::timed_kv(
        "startup.phase",
        &[crate::debug_log::field("phase", "config_load")],
        config::load,
    );

    if args.character.is_some() != args.persona.is_some() {
        anyhow::bail!(
            "The -c (character) and -p (persona) flags must be used together for roleplay mode."
        );
    }

    let api_url = args.api_url.as_deref().unwrap_or_else(|| cfg.api_url());
    let tls_skip_verify = if args.tls_skip_verify {
        true
    } else {
        cfg.tls_skip_verify
    };
    let client = ApiClient::new(api_url, tls_skip_verify);

    let preset_name = args
        .template
        .as_deref()
        .or(cfg.instruct_preset.as_deref())
        .or(cfg.template.as_deref())
        .unwrap_or("Mistral V3-Tekken");
    let instruct_preset = preset::resolve_instruct_preset(preset_name);
    let template_preset_name = cfg.template_preset.as_deref().unwrap_or("Default");
    let template_preset = preset::resolve_template_preset(template_preset_name);

    let sampling = sampling::SamplingParams::default()
        .with_overrides(&cfg.sampling)
        .with_overrides(&args.sampling_overrides());

    let (mut session, mut save_mode) = crate::debug_log::timed_result(
        "startup.phase",
        &[crate::debug_log::field("phase", "resolve_session")],
        || resolve_session(&args),
    )?;

    session.template = Some(instruct_preset.name.clone());

    {
        let content_key = save_mode.key();

        if let Some(ref persona_name) = args.persona {
            session.persona = Some(persona_name.clone());
        } else if session.persona.is_none() && session.tree.head().is_none() {
            session.persona = cfg.default_persona.clone();
        }

        if let Some(ref sp) = args.system_prompt {
            session.system_prompt = Some(sp.clone());
        } else if session.system_prompt.is_none() {
            session.system_prompt = system_prompt::load_prompt_content(
                &config::system_prompts_dir(),
                system_prompt::BUILTIN_ASSISTANT,
                content_key,
            );
        }

        if let Some(ref char_arg) = args.character {
            let card = crate::debug_log::timed_result(
                "startup.phase",
                &[
                    crate::debug_log::field("phase", "resolve_character"),
                    crate::debug_log::field("character", char_arg),
                ],
                || resolve_character(char_arg, content_key),
            )?;
            session.system_prompt = Some(character::build_system_prompt(&card, Some(&template_preset)));
            session.character = Some(card.name.clone());
            if session.tree.head().is_none() && !card.first_mes.is_empty() {
                session
                    .tree
                    .push(None, Message::new(Role::Assistant, card.first_mes.clone()));
            }
        }
    }

    if args.character.is_some() {
        let char_path = config::sessions_dir().join(session::generate_session_name());
        save_mode.set_path(char_path);
    }

    if let Some(ref message) = args.message {
        let text = if message == "-" {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            message.clone()
        };

        let effective_prompt =
            tui::build_effective_system_prompt_standalone(&session, save_mode.key());

        let parent = session.tree.head();
        session.tree.push(parent, Message::new(Role::User, text));

        let branch_path = session.tree.branch_path();
        let prompt_text = instruct_preset.render(&branch_path, effective_prompt.as_deref());
        let stop_tokens = instruct_preset.stop_tokens();
        let stop_refs: Vec<&str> = stop_tokens.iter().map(String::as_str).collect();
        let mut stdout = io::stdout().lock();
        let response = client
            .stream_completion(&prompt_text, &stop_refs, &sampling, &mut stdout)
            .await?;
        writeln!(stdout)?;

        let user_node = session.tree.head().unwrap();
        session
            .tree
            .push(Some(user_node), Message::new(Role::Assistant, response));

        session.maybe_save(&save_mode)?;

        if let Some(path) = save_mode.path() {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                eprintln!("Session: {stem}");
            }
        }

        return Ok(());
    }

    crate::debug_log::log_kv(
        "startup.phase",
        &[
            crate::debug_log::field("phase", "tui_handoff"),
            crate::debug_log::field("mode", "interactive"),
        ],
    );
    let cli_overrides = args.cli_overrides();
    tui::run(
        &client,
        &mut session,
        save_mode,
        instruct_preset,
        sampling,
        cli_overrides,
    )
    .await
}

fn infer_run_mode(args: &Args) -> &'static str {
    if args.cleanup {
        "cleanup"
    } else if let Some(command) = &args.command {
        match command {
            cli::Command::Edit { .. } => "edit_subcommand",
            cli::Command::Update { .. } => "update_subcommand",
        }
    } else if args.message.is_some() {
        "single_message"
    } else {
        "tui"
    }
}

fn build_run_fields(args: &Args) -> Vec<crate::debug_log::Field<'static>> {
    let mut fields = Vec::new();
    fields.push(crate::debug_log::field("has_message", args.message.is_some()));
    fields.push(crate::debug_log::field(
        "message_from_stdin",
        args.message.as_deref() == Some("-"),
    ));
    fields.push(crate::debug_log::field("has_data_dir", args.data.is_some()));
    fields.push(crate::debug_log::field(
        "has_continue",
        args.continue_session.is_some(),
    ));
    fields.push(crate::debug_log::field("no_encrypt", args.no_encrypt));
    fields.push(crate::debug_log::field(
        "has_passkey_arg",
        args.passkey.is_some(),
    ));
    fields.push(crate::debug_log::field(
        "has_system_prompt_arg",
        args.system_prompt.is_some(),
    ));
    fields.push(crate::debug_log::field(
        "has_character_arg",
        args.character.is_some(),
    ));
    fields.push(crate::debug_log::field(
        "has_persona_arg",
        args.persona.is_some(),
    ));
    fields.push(crate::debug_log::field(
        "has_api_url_arg",
        args.api_url.is_some(),
    ));
    fields.push(crate::debug_log::field(
        "has_template_arg",
        args.template.is_some(),
    ));
    fields.push(crate::debug_log::field(
        "timings_enabled",
        args.timings.is_some(),
    ));
    fields.push(crate::debug_log::field(
        "tls_skip_verify",
        args.tls_skip_verify,
    ));

    if let Some(command) = &args.command {
        let command_name = match command {
            cli::Command::Edit { .. } => "edit",
            cli::Command::Update { .. } => "update",
        };
        fields.push(crate::debug_log::field("command", command_name));
        if let cli::Command::Edit { kind, name } = command {
            fields.push(crate::debug_log::field("edit_kind", kind));
            fields.push(crate::debug_log::field("edit_name", name));
        }
    }

    fields
}

fn resolve_session(args: &Args) -> Result<(session::Session, SaveMode)> {
    if args.message.is_some() && args.data.is_none() {
        return Ok((session::Session::default(), SaveMode::None));
    }

    if args.message.is_some() && args.data.is_some() {
        return resolve_persistent_single_shot(args);
    }

    if args.no_encrypt {
        let path = config::sessions_dir().join(session::generate_session_name());
        return Ok((session::Session::default(), SaveMode::Plaintext(path)));
    }

    if let Some(ref passkey) = args.passkey {
        let salt = crypto::load_or_create_salt(&config::salt_path())?;
        let key = crypto::derive_key(passkey, &salt)?;
        let valid = crypto::verify_or_set_key(&config::key_check_path(), &key)?;
        if !valid {
            anyhow::bail!("Wrong passkey.");
        }
        let key = Arc::new(key);
        let path = config::sessions_dir().join(session::generate_session_name());
        return Ok((
            session::Session::default(),
            SaveMode::Encrypted { path, key },
        ));
    }

    let path = config::sessions_dir().join(session::generate_session_name());
    Ok((session::Session::default(), SaveMode::PendingPasskey(path)))
}

fn resolve_persistent_single_shot(args: &Args) -> Result<(session::Session, SaveMode)> {
    let encrypted_key = if let Some(ref passkey) = args.passkey {
        let salt = crypto::load_or_create_salt(&config::salt_path())?;
        let key = crypto::derive_key(passkey, &salt)?;
        let valid = crypto::verify_or_set_key(&config::key_check_path(), &key)?;
        if !valid {
            anyhow::bail!("Wrong passkey.");
        }
        Some(Arc::new(key))
    } else {
        None
    };

    if let Some(ref uuid) = args.continue_session {
        let filename = format!("{uuid}.session");
        let path = config::sessions_dir().join(&filename);
        if !path.exists() {
            anyhow::bail!("Session not found: {uuid}");
        }
        let session = if let Some(ref key) = encrypted_key {
            session::load_encrypted(&path, key)?
        } else {
            session::load(&path)?
        };
        let save_mode = if let Some(key) = encrypted_key {
            SaveMode::Encrypted { path, key }
        } else {
            SaveMode::Plaintext(path)
        };
        return Ok((session, save_mode));
    }

    let path = config::sessions_dir().join(session::generate_session_name());
    let save_mode = if let Some(key) = encrypted_key {
        SaveMode::Encrypted { path, key }
    } else {
        SaveMode::Plaintext(path)
    };
    Ok((session::Session::default(), save_mode))
}

fn resolve_character(
    char_arg: &str,
    key: Option<&crypto::DerivedKey>,
) -> Result<character::CharacterCard> {
    let path = std::path::Path::new(char_arg);
    if path.exists() {
        let card = character::import_card(path)?;
        character::save_card(&card, &config::characters_dir(), key)?;
        return Ok(card);
    }

    let card_path = character::resolve_card_path(&config::characters_dir(), char_arg);
    if !card_path.exists() {
        let report = character::auto_import_png_cards(&config::characters_dir(), key);
        for warning in report.warnings {
            eprintln!("{warning}");
        }
    }
    let card_path = character::resolve_card_path(&config::characters_dir(), char_arg);
    character::load_card(&card_path, key)
}

fn resolve_edit_key(args: &Args) -> Result<Option<Arc<crypto::DerivedKey>>> {
    if args.no_encrypt {
        return Ok(None);
    }

    let passkey = args.passkey.clone().or_else(|| {
        eprint!("Passkey: ");
        rpassword::read_password().ok()
    });

    let Some(passkey) = passkey else {
        anyhow::bail!(
            "No passkey provided. Use --passkey, LIBLLM_PASSKEY, or enter interactively."
        );
    };

    let salt = crypto::load_or_create_salt(&config::salt_path())?;
    let key = crypto::derive_key(&passkey, &salt)?;
    let valid = crypto::verify_or_set_key(&config::key_check_path(), &key)?;
    if !valid {
        anyhow::bail!("Wrong passkey.");
    }
    Ok(Some(Arc::new(key)))
}

fn handle_edit_command(kind: &str, name: &str, args: &Args) -> Result<()> {
    let key = resolve_edit_key(args)?;
    let key_ref = key.as_deref();

    let (json_content, file_path) = match kind {
        "character" | "char" => {
            let card_path = character::resolve_card_path(&config::characters_dir(), name);
            let card = character::load_card(&card_path, key_ref)?;
            let json = serde_json::to_string_pretty(&card)?;
            (json, card_path)
        }
        "worldbook" | "book" | "wb" => {
            let wb_path = worldinfo::resolve_worldbook_path(&config::worldinfo_dir(), name);
            let wb = worldinfo::load_worldbook(&wb_path, key_ref)?;
            let json = serde_json::to_string_pretty(&wb)?;
            (json, wb_path)
        }
        _ => anyhow::bail!("Unknown content type: {kind}. Use 'character' or 'worldbook'."),
    };

    let temp_dir = crate::config::data_dir();
    let temp_path = temp_dir.join(format!(".edit-{name}.json"));

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&temp_path)?;
    file.write_all(json_content.as_bytes())?;
    drop(file);

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_owned());
    let status = std::process::Command::new(&editor)
        .arg(&temp_path)
        .status()?;

    if !status.success() {
        let _ = std::fs::remove_file(&temp_path);
        anyhow::bail!("Editor exited with non-zero status");
    }

    let edited = std::fs::read_to_string(&temp_path)?;
    let _ = std::fs::remove_file(&temp_path);

    match kind {
        "character" | "char" => {
            let card: character::CharacterCard = serde_json::from_str(&edited)
                .map_err(|e| anyhow::anyhow!("Invalid character JSON: {e}"))?;
            let old_path = file_path;
            let new_path = character::save_card(&card, &config::characters_dir(), key_ref)?;
            if new_path != old_path {
                if old_path.exists() {
                    std::fs::remove_file(&old_path).context(format!(
                        "failed to remove old character file: {}",
                        old_path.display()
                    ))?;
                }
                index::warn_if_save_fails(
                    index::remove_character(&old_path, key_ref),
                    "failed to remove character index entry",
                );
            }
            eprintln!("Saved character: {}", card.name);
        }
        "worldbook" | "book" | "wb" => {
            let wb: worldinfo::WorldBook = serde_json::from_str(&edited)
                .map_err(|e| anyhow::anyhow!("Invalid worldbook JSON: {e}"))?;
            let old_path = file_path;
            let new_path = worldinfo::save_worldbook(&wb, &config::worldinfo_dir(), key_ref)?;
            if new_path != old_path {
                if old_path.exists() {
                    std::fs::remove_file(&old_path).context(format!(
                        "failed to remove old worldbook file: {}",
                        old_path.display()
                    ))?;
                }
                index::warn_if_save_fails(
                    index::remove_worldbook(&old_path, key_ref),
                    "failed to remove worldbook index entry",
                );
            }
            eprintln!("Saved worldbook: {}", wb.name);
        }
        _ => unreachable!(),
    }

    Ok(())
}
