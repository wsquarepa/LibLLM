use libllm_core::character;
use libllm_core::client;
use libllm_core::config;
use libllm_core::crypto;
use libllm_core::db::Database;
use libllm_core::debug_log;
use libllm_core::migration;
use libllm_core::preset;
use libllm_core::sampling;
use libllm_core::session;

use libllm::cli;
use libllm::tui;
use libllm::update;

use std::io::{self, IsTerminal, Read, Write};

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
                anyhow::bail!(
                    "--data path exists but is not a directory: {}",
                    data_path.display()
                );
            }
            let is_empty = std::fs::read_dir(data_path)
                .with_context(|| {
                    format!("failed to read --data directory: {}", data_path.display())
                })?
                .next()
                .is_none();
            if !is_empty {
                let has_config =
                    data_path.join("config.toml").exists() || data_path.join("sessions").exists();
                if !has_config {
                    anyhow::bail!(
                        "--data directory is not empty and does not appear to be a libllm data directory: {}",
                        data_path.display()
                    );
                }
            }
            !is_empty
        } else {
            std::fs::create_dir_all(data_path).with_context(|| {
                format!("failed to create --data directory: {}", data_path.display())
            })?;
            false
        };
        config::set_data_dir(data_path.clone())?;

        if is_existing_dir {
            let is_encrypted_dir = config::key_check_path().exists();
            if is_encrypted_dir && args.no_encrypt {
                anyhow::bail!("Data directory is encrypted; --no-encrypt cannot be used with it.");
            }
            if !is_encrypted_dir && args.passkey.is_some() {
                anyhow::bail!("Data directory is not encrypted; --passkey cannot be used with it.");
            }
        }
    }

    let diagnostics_needed =
        args.debug.is_some() || args.timings.is_some() || config::load().debug_log;
    let _diagnostics = if diagnostics_needed {
        Some(debug_log::init(
            args.debug.as_deref(),
            args.timings.as_deref(),
            infer_run_mode(&args),
            &build_run_fields(&args),
        )?)
    } else {
        None
    };

    debug_log::timed_result(
        "startup.phase",
        &[debug_log::field("phase", "ensure_dirs")],
        config::ensure_dirs,
    )?;

    if let Some(cli::Command::Update { branch, list, yes }) = &args.command {
        return update::run(branch.clone(), *list, *yes).await;
    }

    migration::migrate_config_path();

    check_and_run_migration(&args).await?;

    if let Some(cli::Command::Edit { kind, name }) = &args.command {
        return handle_edit_command(kind, name, &args);
    }

    let cfg = debug_log::timed_kv(
        "startup.phase",
        &[debug_log::field("phase", "config_load")],
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

    let (mut session, mut save_mode, mut db) = debug_log::timed_result(
        "startup.phase",
        &[debug_log::field("phase", "resolve_session")],
        || resolve_session(&args),
    )?;

    session.template = Some(instruct_preset.name.clone());

    {
        if let Some(ref persona_name) = args.persona {
            session.persona = Some(persona_name.clone());
        } else if session.persona.is_none() && session.tree.head().is_none() {
            session.persona = cfg.default_persona.clone();
        }

        if let Some(ref sp) = args.system_prompt {
            session.system_prompt = Some(sp.clone());
        } else if session.system_prompt.is_none() {
            if let Some(ref db) = db {
                session.system_prompt = db
                    .load_prompt(libllm_core::system_prompt::BUILTIN_ASSISTANT)
                    .ok()
                    .map(|p| p.content);
            }
        }

        if let Some(ref char_arg) = args.character {
            let card = debug_log::timed_result(
                "startup.phase",
                &[
                    debug_log::field("phase", "resolve_character"),
                    debug_log::field("character", char_arg),
                ],
                || resolve_character(char_arg, db.as_ref()),
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
        let text = if message == "-" {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            message.clone()
        };

        let effective_prompt =
            tui::build_effective_system_prompt_standalone(&session, db.as_ref());

        let parent = session.tree.head();
        let user_node = session.tree.push(parent, Message::new(Role::User, text));

        let branch_path = session.tree.branch_path();
        let prompt_text = instruct_preset.render(&branch_path, effective_prompt.as_deref());
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

        return Ok(());
    }

    debug_log::log_kv(
        "startup.phase",
        &[
            debug_log::field("phase", "tui_handoff"),
            debug_log::field("mode", "interactive"),
        ],
    );
    let cli_overrides = args.cli_overrides();
    tui::run(
        client,
        &mut session,
        save_mode,
        db,
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

fn build_run_fields(args: &Args) -> Vec<debug_log::Field<'static>> {
    let mut fields = vec![
        debug_log::field("has_message", args.message.is_some()),
        debug_log::field("message_from_stdin", args.message.as_deref() == Some("-")),
        debug_log::field("has_data_dir", args.data.is_some()),
        debug_log::field("has_continue", args.continue_session.is_some()),
        debug_log::field("no_encrypt", args.no_encrypt),
        debug_log::field("has_passkey_arg", args.passkey.is_some()),
        debug_log::field("has_system_prompt_arg", args.system_prompt.is_some()),
        debug_log::field("has_character_arg", args.character.is_some()),
        debug_log::field("has_persona_arg", args.persona.is_some()),
        debug_log::field("has_api_url_arg", args.api_url.is_some()),
        debug_log::field("has_template_arg", args.template.is_some()),
        debug_log::field("timings_enabled", args.timings.is_some()),
        debug_log::field("tls_skip_verify", args.tls_skip_verify),
    ];

    if let Some(command) = &args.command {
        let command_name = match command {
            cli::Command::Edit { .. } => "edit",
            cli::Command::Update { .. } => "update",
        };
        fields.push(debug_log::field("command", command_name));
        if let cli::Command::Edit { kind, name } = command {
            fields.push(debug_log::field("edit_kind", kind));
            fields.push(debug_log::field("edit_name", name));
        }
    }

    fields
}

fn resolve_session(args: &Args) -> Result<(session::Session, SaveMode, Option<Database>)> {
    if args.message.is_some() && args.data.is_none() {
        return Ok((session::Session::default(), SaveMode::None, None));
    }

    let db_path = config::data_dir().join("data.db");

    if args.no_encrypt {
        let db = Database::open(&db_path, None)?;
        db.ensure_builtin_prompts()?;
        let id = session::generate_session_id();
        if let Some(ref uuid) = args.continue_session {
            let session = db.load_session(uuid)?;
            return Ok((session, SaveMode::Database { id: uuid.clone() }, Some(db)));
        }
        return Ok((session::Session::default(), SaveMode::Database { id }, Some(db)));
    }

    if let Some(ref passkey) = args.passkey {
        let salt = crypto::load_or_create_salt(&config::salt_path())?;
        let key = crypto::derive_key(passkey, &salt)?;
        let valid = crypto::verify_or_set_key(&config::key_check_path(), &key)?;
        if !valid {
            anyhow::bail!("Wrong passkey.");
        }
        let db = Database::open(&db_path, Some(&key))?;
        db.ensure_builtin_prompts()?;
        let id = session::generate_session_id();
        if let Some(ref uuid) = args.continue_session {
            let session = db.load_session(uuid)?;
            return Ok((session, SaveMode::Database { id: uuid.clone() }, Some(db)));
        }
        return Ok((session::Session::default(), SaveMode::Database { id }, Some(db)));
    }

    let id = session::generate_session_id();
    Ok((session::Session::default(), SaveMode::PendingPasskey { id }, None))
}

fn resolve_character(
    char_arg: &str,
    db: Option<&Database>,
) -> Result<character::CharacterCard> {
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
    if let Some(db) = db {
        if let Ok(card) = db.load_character(&slug) {
            return Ok(card);
        }
    }

    anyhow::bail!("Character not found: {char_arg}");
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
    let valid = crypto::verify_or_set_key(&config::key_check_path(), &key)?;
    if !valid {
        anyhow::bail!("Wrong passkey.");
    }
    Database::open(&db_path, Some(&key))
}

fn handle_edit_command(kind: &str, name: &str, args: &Args) -> Result<()> {
    let db = resolve_edit_db(args)?;

    let slug = character::slugify(name);
    let json_content = match kind {
        "character" | "char" => {
            let card = db.load_character(&slug)?;
            serde_json::to_string_pretty(&card)?
        }
        "worldbook" | "book" | "wb" => {
            let wb = db.load_worldbook(&slug)?;
            serde_json::to_string_pretty(&wb)?
        }
        _ => anyhow::bail!("Unknown content type: {kind}. Use 'character' or 'worldbook'."),
    };

    let temp_dir = config::data_dir();
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
            let new_slug = character::slugify(&card.name);
            if new_slug != slug {
                let _ = db.delete_character(&slug);
            }
            if db.load_character(&new_slug).is_ok() {
                db.update_character(&new_slug, &card)?;
            } else {
                db.insert_character(&new_slug, &card)?;
            }
            eprintln!("Saved character: {}", card.name);
        }
        "worldbook" | "book" | "wb" => {
            let wb: libllm_core::worldinfo::WorldBook = serde_json::from_str(&edited)
                .map_err(|e| anyhow::anyhow!("Invalid worldbook JSON: {e}"))?;
            let new_slug = character::slugify(&wb.name);
            if new_slug != slug {
                let _ = db.delete_worldbook(&slug);
            }
            if db.load_worldbook(&new_slug).is_ok() {
                db.update_worldbook(&new_slug, &wb)?;
            } else {
                db.insert_worldbook(&new_slug, &wb)?;
            }
            eprintln!("Saved worldbook: {}", wb.name);
        }
        _ => unreachable!(),
    }

    Ok(())
}

const LEGACY_DIRS: [&str; 5] = ["sessions", "characters", "worldinfo", "system", "personas"];

async fn check_and_run_migration(args: &Args) -> Result<()> {
    let data_dir = config::data_dir();
    let db_path = data_dir.join("data.db");

    if db_path.exists() {
        return Ok(());
    }

    let has_legacy = LEGACY_DIRS.iter().any(|dir| {
        let path = data_dir.join(dir);
        path.is_dir()
            && std::fs::read_dir(&path)
                .map(|mut d| d.next().is_some())
                .unwrap_or(false)
    });

    if !has_legacy {
        return Ok(());
    }

    eprintln!("Legacy file-based data detected. Migration to SQLite is required.");

    let migrate_name = if cfg!(target_os = "windows") {
        "migrate.exe"
    } else {
        "migrate"
    };

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_owned()));

    let migrate_path = exe_dir
        .as_ref()
        .map(|d| d.join(migrate_name))
        .filter(|p| p.exists());

    let (migrate_path, was_downloaded) = if let Some(path) = migrate_path {
        (path, false)
    } else {
        eprintln!("Migration utility not found.");

        let channel = update::CHANNEL;
        if channel == "unknown" {
            anyhow::bail!(
                "Legacy data directory needs migration but no '{}' binary found.\n\
                 Build it with: cargo build -p libllm-migrate\n\
                 Then run: libllm-migrate -d {}",
                migrate_name,
                data_dir.display()
            );
        }

        let stdin = io::stdin();
        if !stdin.is_terminal() {
            anyhow::bail!(
                "Legacy data directory needs migration. Download the migration utility from\n\
                 https://github.com/{}/releases/tag/{} or run in an interactive terminal.",
                update::REPO,
                channel
            );
        }

        eprint!("Download migration utility from GitHub? [Y/n] ");
        io::stderr().flush()?;
        let mut answer = String::new();
        stdin.read_line(&mut answer)?;
        if !answer.trim().is_empty() && !answer.trim().eq_ignore_ascii_case("y") {
            anyhow::bail!("Migration required. Cannot continue without migrating data.");
        }

        let dest = exe_dir
            .as_ref()
            .map(|d| d.join(migrate_name))
            .unwrap_or_else(|| std::path::PathBuf::from(migrate_name));

        download_migrate_binary(&dest).await?;
        (dest, true)
    };

    eprintln!("Running migration...");
    let mut cmd = std::process::Command::new(&migrate_path);
    cmd.arg("-d").arg(&data_dir);

    if args.no_encrypt {
        cmd.arg("--no-encrypt");
    }
    if let Some(ref passkey) = args.passkey {
        cmd.arg("--passkey").arg(passkey);
    }

    let status = cmd.status().context("failed to run migration utility")?;

    if was_downloaded {
        let _ = std::fs::remove_file(&migrate_path);
    }

    if !status.success() {
        anyhow::bail!(
            "Migration failed with exit code: {}",
            status.code().unwrap_or(-1)
        );
    }

    eprintln!("Migration complete.");
    Ok(())
}

async fn download_migrate_binary(dest: &std::path::Path) -> Result<()> {
    let channel = update::CHANNEL;
    let client = update::build_client()?;

    let url = format!(
        "https://api.github.com/repos/{}/releases/tags/{}",
        update::REPO,
        channel
    );
    let release = update::fetch_release(&client, &url).await?;

    let expected_name = if cfg!(target_os = "windows") {
        format!("migrate-{}.exe", update::TARGET)
    } else {
        format!("migrate-{}", update::TARGET)
    };

    let asset = release
        .assets
        .iter()
        .find(|a| a.name == expected_name)
        .context(format!(
            "no migration utility found for this platform ({}) in the {} release",
            update::TARGET,
            channel
        ))?;

    eprintln!("Downloading {}...", asset.name);

    let resp = client
        .get(&asset.url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .context("failed to download migration utility")?;

    if !resp.status().is_success() {
        let status = resp.status();
        anyhow::bail!("download failed with status {status}");
    }

    let bytes = resp.bytes().await.context("failed to read download")?;
    std::fs::write(dest, &bytes).context("failed to write migration utility")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))
            .context("failed to set executable permissions")?;
    }

    eprintln!("Saved to {}", dest.display());
    Ok(())
}
