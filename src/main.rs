mod character;
mod cli;
mod client;
mod commands;
mod config;
mod context;
mod crypto;
mod debug_log;
mod prompt;
mod sampling;
mod session;
mod tui;
mod worldinfo;

use std::io::{self, Read, Write};
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;

use cli::Args;
use client::ApiClient;
use prompt::Template;
use session::{Message, Role, SaveMode};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    #[cfg(debug_assertions)]
    if let Some(ref path) = args.debug {
        debug_log::init(path);
    }
    config::ensure_dirs()?;

    if let Some(cli::Command::Edit { kind, name }) = &args.command {
        return handle_edit_command(kind, name, &args);
    }

    let cfg = config::load();

    let api_url = args.api_url.as_deref().unwrap_or_else(|| cfg.api_url());
    let client = ApiClient::new(api_url);

    let template_name = args
        .template
        .as_deref()
        .or(cfg.template.as_deref())
        .unwrap_or("llama2");
    let template = Template::from_name(template_name);

    let sampling = sampling::SamplingParams::default()
        .with_overrides(&cfg.sampling)
        .with_overrides(&args.sampling_overrides());

    let (mut session, mut save_mode) = resolve_session(&args)?;

    let content_key = save_mode.key();
    for warning in character::auto_import_png_cards(&config::characters_dir(), content_key) {
        eprintln!("{warning}");
    }
    for warning in worldinfo::normalize_worldbooks(&config::worldinfo_dir(), content_key) {
        eprintln!("{warning}");
    }
    if let Some(key) = content_key {
        for warning in character::encrypt_plaintext_cards(&config::characters_dir(), key) {
            eprintln!("{warning}");
        }
    }

    session.template = Some(template.name().to_owned());

    if session.system_prompt.is_none() {
        session.system_prompt = args.system_prompt.or(cfg.system_prompt);
    }

    if let Some(ref char_arg) = args.character {
        let card = resolve_character(char_arg, content_key)?;
        session.system_prompt = Some(character::build_system_prompt(&card));
        session.character = Some(card.name.clone());
        if session.tree.head().is_none() && !card.first_mes.is_empty() {
            session
                .tree
                .push(None, Message::new(Role::Assistant, card.first_mes.clone()));
        }
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

        let parent = session.tree.head();
        session.tree.push(parent, Message::new(Role::User, text));

        let branch_path = session.tree.branch_path();
        let prompt_text = template.render(&branch_path, session.system_prompt.as_deref());
        let stop_tokens = template.stop_tokens();
        let mut stdout = io::stdout().lock();
        let response = client
            .stream_completion(&prompt_text, stop_tokens, &sampling, &mut stdout)
            .await?;
        writeln!(stdout)?;

        let user_node = session.tree.head().unwrap();
        session
            .tree
            .push(Some(user_node), Message::new(Role::Assistant, response));

        session.maybe_save(&save_mode)?;

        return Ok(());
    }

    tui::run(&client, &mut session, save_mode, template, sampling).await
}

fn resolve_session(args: &Args) -> Result<(session::Session, SaveMode)> {
    if let Some(ref path) = args.session {
        let session = session::load(path)?;
        return Ok((session, SaveMode::Plaintext(path.clone())));
    }

    if args.message.is_some() {
        return Ok((session::Session::default(), SaveMode::None));
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

    let temp_dir = std::env::temp_dir();
    let temp_path = temp_dir.join(format!("libllm-edit-{name}.json"));

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
            if new_path != old_path && old_path.exists() {
                let _ = std::fs::remove_file(&old_path);
            }
            eprintln!("Saved character: {}", card.name);
        }
        "worldbook" | "book" | "wb" => {
            let wb: worldinfo::WorldBook = serde_json::from_str(&edited)
                .map_err(|e| anyhow::anyhow!("Invalid worldbook JSON: {e}"))?;
            let old_path = file_path;
            let new_path = worldinfo::save_worldbook(&wb, &config::worldinfo_dir(), key_ref)?;
            if new_path != old_path && old_path.exists() {
                let _ = std::fs::remove_file(&old_path);
            }
            eprintln!("Saved worldbook: {}", wb.name);
        }
        _ => unreachable!(),
    }

    Ok(())
}
