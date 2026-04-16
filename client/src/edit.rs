//! External editor integration for character cards and worldbooks.

use anyhow::Result;
use libllm::character;
use libllm::config;
use libllm::db::Database;
use std::io::Write;

pub fn handle_edit_command(kind: &str, name: &str, db: &Database) -> Result<()> {
    libllm::debug_log::log_kv(
        "edit.run",
        &[
            libllm::debug_log::field("kind", kind),
            libllm::debug_log::field("name", name),
        ],
    );
    let slug = character::slugify(name);
    let json_content: String = libllm::debug_log::timed_result(
        "edit.phase",
        &[libllm::debug_log::field("phase", "load_content")],
        || -> Result<String> {
            match kind {
                "character" | "char" => {
                    let card = db.load_character(&slug)?;
                    Ok(serde_json::to_string_pretty(&card)?)
                }
                "worldbook" | "book" | "wb" => {
                    let wb = db.load_worldbook(&slug)?;
                    Ok(serde_json::to_string_pretty(&wb)?)
                }
                _ => {
                    anyhow::bail!("Unknown content type: {kind}. Use 'character' or 'worldbook'.")
                }
            }
        },
    )?;

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
    libllm::debug_log::log_kv(
        "edit.phase",
        &[
            libllm::debug_log::field("phase", "write_temp"),
            libllm::debug_log::field("path", temp_path.display()),
            libllm::debug_log::field("bytes", json_content.len()),
        ],
    );

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_owned());
    let status = libllm::debug_log::timed_result(
        "edit.phase",
        &[
            libllm::debug_log::field("phase", "spawn_editor"),
            libllm::debug_log::field("editor", &editor),
        ],
        || std::process::Command::new(&editor).arg(&temp_path).status(),
    )?;

    if !status.success() {
        let _ = std::fs::remove_file(&temp_path);
        anyhow::bail!("Editor exited with non-zero status");
    }

    let edited = std::fs::read_to_string(&temp_path)?;
    let _ = std::fs::remove_file(&temp_path);

    libllm::debug_log::timed_result(
        "edit.phase",
        &[libllm::debug_log::field("phase", "save")],
        || -> Result<()> {
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
                    let wb: libllm::worldinfo::WorldBook = serde_json::from_str(&edited)
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
        },
    )?;

    Ok(())
}
