use libllm::character;
use libllm::client::ApiClient;
use libllm::config;
use libllm::crypto;
use libllm::db::Database;
use libllm::diagnostics;
use libllm::migration;
use libllm::preset;
use libllm::sampling;
use libllm::session;

use client::cli;
use client::edit;
use client::import;
use client::legacy_migration;
use client::recover;
use client::tui;
use client::update;
use client::validation;

use std::io::{self, IsTerminal, Read, Write};

use anyhow::{Context, Result};
use clap::Parser;

use cli::Args;
use session::{Message, Role, SaveMode};

#[tokio::main]
async fn main() -> Result<()> {
    libllm::crypto_provider::install_default_crypto_provider();
    let args = Args::parse();

    if args.version {
        println!("{}", client::version::LONG);
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
            use crossterm::execute;
            use crossterm::style::{
                Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor,
            };

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

    if let Some(ref data_path) = args.data {
        let is_existing_dir = validation::validate_data_dir(data_path, args.no_encrypt)?;
        config::set_data_dir(data_path.clone())?;

        if is_existing_dir {
            let is_encrypted_dir = config::salt_path().exists();
            if is_encrypted_dir && args.no_encrypt {
                anyhow::bail!("Data directory is encrypted; --no-encrypt cannot be used with it.");
            }
            if !is_encrypted_dir && args.passkey.is_some() {
                anyhow::bail!("Data directory is not encrypted; --passkey cannot be used with it.");
            }
        }
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

    libllm::timed_result!(
        tracing::Level::INFO,
        "startup.phase",
        phase = "ensure_dirs" ;
        { config::ensure_dirs() }
    )?;

    if let Some(cli::Command::Update { branch, yes }) = &args.command {
        return update::run(branch.clone(), *yes).await;
    }

    if let Some(cli::Command::Recover { command }) = &args.command {
        let data_dir = args
            .data
            .as_deref()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(config::data_dir);
        let passkey = resolve_recover_passkey(&args, &data_dir)?;
        return recover::run(&data_dir, passkey.as_deref(), command.as_ref());
    }

    if let Some(cli::Command::Db { command }) = &args.command {
        return cli::db::dispatch(&args, command);
    }

    migration::migrate_config_path();

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
        config::load()
    };

    let api_url = args.api_url.as_deref().unwrap_or_else(|| cfg.api_url());
    let tls_skip_verify = if args.tls_skip_verify {
        true
    } else {
        cfg.tls_skip_verify
    };
    if tls_skip_verify {
        use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
        let _ = crossterm::execute!(
            io::stderr(),
            SetForegroundColor(Color::Yellow),
            Print("Warning: TLS certificate verification is disabled.\n"),
            ResetColor,
        );
    }
    let cli_overrides = args.cli_overrides();
    let auth = libllm::config::resolve_auth(&cfg, &cli_overrides.auth_overrides());
    let client = ApiClient::new(api_url, tls_skip_verify, auth);

    let preset_name = args
        .template
        .as_deref()
        .or(cfg.instruct_preset.as_deref())
        .unwrap_or("Mistral V3-Tekken");
    let instruct_preset = preset::resolve_instruct_preset(preset_name);
    let reasoning_preset = cfg
        .reasoning_preset
        .as_deref()
        .and_then(preset::resolve_reasoning_preset);
    let template_preset_name = cfg.template_preset.as_deref().unwrap_or("Default");
    let template_preset = preset::resolve_template_preset(template_preset_name);

    let sampling = sampling::SamplingParams::default()
        .with_overrides(&cfg.sampling)
        .with_overrides(&args.sampling_overrides());

    let (mut session, mut save_mode, mut db, summarizer_db_path, summarizer_key) =
        libllm::timed_result!(
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
                .load_prompt(libllm::system_prompt::BUILTIN_ASSISTANT)
                .ok()
                .map(|p| p.content);
        }

        if let Some(ref char_arg) = args.character {
            let card = libllm::timed_result!(
                tracing::Level::INFO,
                "startup.phase",
                phase = "resolve_character",
                character = char_arg.as_str() ;
                { resolve_character(char_arg, db.as_ref()) }
            )?;
            session.system_prompt = Some(character::build_system_prompt(
                &card,
                Some(&template_preset),
            ));
            session.character = Some(card.name.clone());
            if session.tree.head().is_none() && !card.first_mes.is_empty() {
                session
                    .tree
                    .push(None, Message::new(Role::Assistant, card.first_mes.clone()));
            }
        }
    }

    if args.character.is_some() {
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
                match libllm::files::stdin_attachment(bytes, &cfg.files) {
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

        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let prepended = stdin_attachment.into_iter().collect::<Vec<_>>();
        let system_messages = match libllm::files::resolve_with_prepended(
            prepended,
            &text,
            &cwd,
            &cfg.files,
        ) {
            Ok(msgs) => msgs,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        };

        let effective_prompt = tui::build_effective_system_prompt_standalone(&session, db.as_ref());

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
                    content: libllm::files::rewrite_user_message(&m.content),
                    timestamp: m.timestamp.clone(),
                    thought_seconds: m.thought_seconds,
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

        session.maybe_save(&save_mode, db.as_mut())?;

        if let Some(id) = save_mode.id() {
            eprintln!("Session: {id}");
        }

        try_backup(&config::data_dir(), args.passkey.as_deref(), &cfg.backup);

        return Ok(());
    }

    tracing::info!(phase = "tui_handoff", mode = "interactive", "startup.phase");
    let resolved_passkey = tui::run(
        client,
        &mut session,
        save_mode,
        db,
        instruct_preset,
        sampling,
        cli_overrides,
        tui::SummarizerParams {
            db_path: summarizer_db_path,
            derived_key: summarizer_key,
        },
    )
    .await?;

    let effective_passkey = resolved_passkey.as_deref().or(args.passkey.as_deref());
    let current_config = config::load();
    try_backup(
        &config::data_dir(),
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
        return Ok((session::Session::default(), SaveMode::None, None, None, None));
    }

    let db_path = config::data_dir().join("data.db");

    if args.no_encrypt {
        let db = Database::open(&db_path, None)?;
        db.ensure_builtin_prompts()?;
        preset::ensure_default_presets();
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
        let salt = crypto::load_or_create_salt(&config::salt_path())?;
        let key = crypto::derive_key(passkey, &salt)?;
        let key_arc = std::sync::Arc::new(key);
        let db = Database::open(&db_path, Some(&*key_arc))
            .context("Wrong passkey (or corrupt database).")?;
        db.ensure_builtin_prompts()?;
        preset::ensure_default_presets();
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
    if !client::interactive::is_interactive() {
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
    let db_path = config::data_dir().join("data.db");

    if args.no_encrypt {
        return Database::open(&db_path, None);
    }

    let passkey = match args.passkey.clone() {
        Some(passkey) => Some(passkey),
        None => {
            eprint!("Passkey: ");
            Some(rpassword::read_password().context("failed to read interactive passkey")?)
        }
    };

    let Some(passkey) = passkey else {
        anyhow::bail!(
            "No passkey provided. Use --passkey, LIBLLM_PASSKEY, or enter interactively."
        );
    };

    let salt = crypto::load_or_create_salt(&config::salt_path())?;
    let key = crypto::derive_key(&passkey, &salt)?;
    Database::open(&db_path, Some(&key)).context("Wrong passkey (or corrupt database).")
}

fn try_backup(
    data_dir: &std::path::Path,
    passkey: Option<&str>,
    config: &libllm::config::BackupConfig,
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

    if let Err(err) = backup::snapshot::create_snapshot(data_dir, passkey, config) {
        eprintln!("Warning: backup failed: {err}");
    }
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

        let config = libllm::config::BackupConfig::default();
        try_backup(data_dir, None, &config);

        assert!(
            !data_dir.join("backups").exists(),
            "try_backup must not touch an encrypted database without a passkey",
        );
    }
}
