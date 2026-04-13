use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use libllm_core::crypto::{derive_key, load_or_create_salt, verify_or_set_key};
use libllm_core::db::Database;
use libllm_core::system_prompt::{BUILTIN_ASSISTANT, BUILTIN_ROLEPLAY};

mod backup;
mod legacy;

#[derive(Parser)]
#[command(
    name = "libllm-migrate",
    about = "Migrate LibLLM data from file-based storage to SQLite database"
)]
struct Args {
    #[arg(short, long)]
    data: Option<PathBuf>,
    #[arg(long)]
    no_encrypt: bool,
    #[arg(long, env = "LIBLLM_PASSKEY")]
    passkey: Option<String>,
}

fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("libllm")
}

fn prompt_passkey() -> Result<String> {
    eprint!("Enter passkey: ");
    let passkey = rpassword::read_password()
        .context("failed to read passkey")?;
    if passkey.is_empty() {
        bail!("passkey cannot be empty");
    }
    Ok(passkey)
}

fn main() -> Result<()> {
    let args = Args::parse();

    let data_dir = args.data.unwrap_or_else(default_data_dir);
    if !data_dir.is_dir() {
        bail!(
            "data directory does not exist: {}",
            data_dir.display()
        );
    }

    let db_path = data_dir.join("data.db");
    if db_path.exists() {
        bail!(
            "database already exists at {} -- migration has already been run or is not needed",
            db_path.display()
        );
    }

    if !legacy::has_legacy_files(&data_dir) {
        bail!(
            "no legacy files found in {} -- nothing to migrate",
            data_dir.display()
        );
    }

    let salt_path = data_dir.join(".salt");
    let key_check_path = data_dir.join(".key_check");
    let encrypted_mode = !args.no_encrypt && salt_path.exists();

    let derived_key = if encrypted_mode {
        let passkey = match args.passkey {
            Some(p) => p,
            None => prompt_passkey()?,
        };
        let salt = load_or_create_salt(&salt_path)?;
        let key = derive_key(&passkey, &salt)?;

        if key_check_path.exists() {
            let valid = verify_or_set_key(&key_check_path, &key)?;
            if !valid {
                bail!("incorrect passkey -- key verification failed");
            }
        }
        Some(key)
    } else {
        None
    };

    let key_bytes: Option<&[u8; 32]> = derived_key.as_ref().map(|k| k.as_bytes());

    eprintln!("Creating backup...");
    let archive_path = backup::create_backup(&data_dir)?;
    eprintln!("  Backup saved to: {}", archive_path.display());

    eprintln!("Creating database...");
    let db = Database::open(&db_path, derived_key.as_ref())?;

    let sessions_dir = data_dir.join("sessions");
    let characters_dir = data_dir.join("characters");
    let worldinfo_dir = data_dir.join("worldinfo");
    let system_dir = data_dir.join("system");
    let personas_dir = data_dir.join("personas");

    let mut session_count = 0usize;
    let mut character_count = 0usize;
    let mut worldbook_count = 0usize;
    let mut persona_count = 0usize;
    let mut prompt_count = 0usize;
    let mut error_count = 0usize;

    let sessions = if sessions_dir.is_dir() {
        eprintln!("Reading sessions...");
        legacy::read_sessions(&sessions_dir, key_bytes)?
    } else {
        Vec::new()
    };

    let characters = if characters_dir.is_dir() {
        eprintln!("Reading characters...");
        legacy::read_characters(&characters_dir, key_bytes)?
    } else {
        Vec::new()
    };

    let worldbooks = if worldinfo_dir.is_dir() {
        eprintln!("Reading worldbooks...");
        legacy::read_worldbooks(&worldinfo_dir, key_bytes)?
    } else {
        Vec::new()
    };

    let personas = if personas_dir.is_dir() {
        eprintln!("Reading personas...");
        legacy::read_personas(&personas_dir, key_bytes)?
    } else {
        Vec::new()
    };

    let prompts = if system_dir.is_dir() {
        eprintln!("Reading system prompts...");
        legacy::read_prompts(&system_dir, key_bytes)?
    } else {
        Vec::new()
    };

    eprintln!("Importing into database...");
    db.in_transaction(|_conn| {
        for (slug, session) in &sessions {
            match db.insert_session(slug, session) {
                Ok(()) => session_count += 1,
                Err(e) => {
                    eprintln!("  warning: failed to import session {slug}: {e}");
                    error_count += 1;
                }
            }
        }

        for (slug, card) in &characters {
            match db.insert_character(slug, card) {
                Ok(()) => character_count += 1,
                Err(e) => {
                    eprintln!("  warning: failed to import character {slug}: {e}");
                    error_count += 1;
                }
            }
        }

        for (slug, book) in &worldbooks {
            match db.insert_worldbook(slug, book) {
                Ok(()) => worldbook_count += 1,
                Err(e) => {
                    eprintln!("  warning: failed to import worldbook {slug}: {e}");
                    error_count += 1;
                }
            }
        }

        for (slug, persona) in &personas {
            match db.insert_persona(slug, persona) {
                Ok(()) => persona_count += 1,
                Err(e) => {
                    eprintln!("  warning: failed to import persona {slug}: {e}");
                    error_count += 1;
                }
            }
        }

        for (slug, prompt) in &prompts {
            let builtin = slug == BUILTIN_ASSISTANT || slug == BUILTIN_ROLEPLAY;
            match db.insert_prompt(slug, prompt, builtin) {
                Ok(()) => prompt_count += 1,
                Err(e) => {
                    eprintln!("  warning: failed to import system prompt {slug}: {e}");
                    error_count += 1;
                }
            }
        }

        Ok(())
    })?;

    eprintln!("Ensuring builtin prompts...");
    db.ensure_builtin_prompts()?;

    eprintln!("Cleaning up old files...");
    for dir in [
        &sessions_dir,
        &characters_dir,
        &worldinfo_dir,
        &system_dir,
        &personas_dir,
    ] {
        if dir.is_dir() {
            if let Err(e) = std::fs::remove_dir_all(dir) {
                eprintln!(
                    "  warning: failed to remove {}: {e}",
                    dir.display()
                );
            }
        }
    }

    let index_meta = data_dir.join("index.meta");
    if index_meta.exists() {
        if let Err(e) = std::fs::remove_file(&index_meta) {
            eprintln!(
                "  warning: failed to remove {}: {e}",
                index_meta.display()
            );
        }
    }

    eprintln!();
    eprintln!("Migration complete:");
    eprintln!("  Sessions:       {session_count}");
    eprintln!("  Characters:     {character_count}");
    eprintln!("  Worldbooks:     {worldbook_count}");
    eprintln!("  Personas:       {persona_count}");
    eprintln!("  System prompts: {prompt_count}");
    if error_count > 0 {
        eprintln!("  Errors:         {error_count}");
    }
    eprintln!("  Database:       {}", db_path.display());
    eprintln!("  Backup:         {}", archive_path.display());

    Ok(())
}
