//! External editor integration for character cards and worldbooks.

use anyhow::Result;
use std::io::Write;
use libllm::character;
use libllm::config;
use libllm::db::Database;

pub fn handle_edit_command(kind: &str, name: &str, db: &Database) -> Result<()> {
    let slug = character::slugify(name);
    let normalized_kind = match kind {
        "character" | "char" => "character",
        "worldbook" | "book" | "wb" => "worldbook",
        _ => "unknown",
    };
    tracing::debug!(phase = "start", kind = normalized_kind, slug = slug.as_str(), "edit.run");

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
    tracing::debug!(phase = "write", result = "ok", path = %temp_path.display(), bytes = json_content.len(), "edit.temp_file");

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_owned());
    let status = libllm::timed_result!(
        tracing::Level::INFO,
        "edit.editor",
        editor = editor.as_str() ;
        {
            std::process::Command::new(&editor)
                .arg(&temp_path)
                .status()
                .map_err(anyhow::Error::from)
        }
    )?;

    if !status.success() {
        let exit_code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "none".to_owned());
        tracing::warn!(phase = "exit", result = "error", exit_code = exit_code.as_str(), "edit.editor");
        let cleanup = std::fs::remove_file(&temp_path);
        tracing::debug!(phase = "cleanup", result = if cleanup.is_ok() { "ok" } else { "error" }, path = %temp_path.display(), "edit.temp_file");
        anyhow::bail!("Editor exited with non-zero status");
    }

    let edited = std::fs::read_to_string(&temp_path)?;
    let cleanup = std::fs::remove_file(&temp_path);
    tracing::debug!(phase = "cleanup", result = if cleanup.is_ok() { "ok" } else { "error" }, path = %temp_path.display(), "edit.temp_file");

    match kind {
        "character" | "char" => {
            let card: character::CharacterCard = serde_json::from_str(&edited)
                .map_err(|e| anyhow::anyhow!("Invalid character JSON: {e}"))?;
            let new_slug = character::slugify(&card.name);
            if new_slug != slug {
                let _ = db.delete_character(&slug);
            }
            let operation = if db.load_character(&new_slug).is_ok() {
                db.update_character(&new_slug, &card)?;
                "update"
            } else {
                db.insert_character(&new_slug, &card)?;
                "insert"
            };
            tracing::info!(kind = "character", slug = slug.as_str(), new_slug = new_slug.as_str(), renamed = new_slug != slug, operation = operation, bytes = edited.len(), result = "ok", "edit.save");
            eprintln!("Saved character: {}", card.name);
        }
        "worldbook" | "book" | "wb" => {
            let wb: libllm::worldinfo::WorldBook = serde_json::from_str(&edited)
                .map_err(|e| anyhow::anyhow!("Invalid worldbook JSON: {e}"))?;
            let new_slug = character::slugify(&wb.name);
            if new_slug != slug {
                let _ = db.delete_worldbook(&slug);
            }
            let operation = if db.load_worldbook(&new_slug).is_ok() {
                db.update_worldbook(&new_slug, &wb)?;
                "update"
            } else {
                db.insert_worldbook(&new_slug, &wb)?;
                "insert"
            };
            tracing::info!(kind = "worldbook", slug = slug.as_str(), new_slug = new_slug.as_str(), renamed = new_slug != slug, operation = operation, bytes = edited.len(), result = "ok", "edit.save");
            eprintln!("Saved worldbook: {}", wb.name);
        }
        _ => unreachable!(),
    }

    Ok(())
}
