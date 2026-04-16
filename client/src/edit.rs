//! External editor integration for character cards and worldbooks.

use anyhow::Result;
use std::io::Write;
use libllm::character;
use libllm::config;
use libllm::db::Database;
use libllm::debug_log;

pub fn handle_edit_command(kind: &str, name: &str, db: &Database) -> Result<()> {
    let slug = character::slugify(name);
    let normalized_kind = match kind {
        "character" | "char" => "character",
        "worldbook" | "book" | "wb" => "worldbook",
        _ => "unknown",
    };
    debug_log::log_kv(
        "edit.run",
        &[
            debug_log::field("phase", "start"),
            debug_log::field("kind", normalized_kind),
            debug_log::field("slug", &slug),
        ],
    );

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
    debug_log::log_kv(
        "edit.temp_file",
        &[
            debug_log::field("phase", "write"),
            debug_log::field("result", "ok"),
            debug_log::field("path", temp_path.display()),
            debug_log::field("bytes", json_content.len()),
        ],
    );

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_owned());
    let status = debug_log::timed_result(
        "edit.editor",
        &[debug_log::field("editor", &editor)],
        || {
            std::process::Command::new(&editor)
                .arg(&temp_path)
                .status()
                .map_err(anyhow::Error::from)
        },
    )?;

    if !status.success() {
        debug_log::log_kv(
            "edit.editor",
            &[
                debug_log::field("phase", "exit"),
                debug_log::field("result", "error"),
                debug_log::field(
                    "exit_code",
                    status.code().map(|c| c.to_string()).unwrap_or_else(|| "none".to_owned()),
                ),
            ],
        );
        let cleanup = std::fs::remove_file(&temp_path);
        debug_log::log_kv(
            "edit.temp_file",
            &[
                debug_log::field("phase", "cleanup"),
                debug_log::field(
                    "result",
                    if cleanup.is_ok() { "ok" } else { "error" },
                ),
                debug_log::field("path", temp_path.display()),
            ],
        );
        anyhow::bail!("Editor exited with non-zero status");
    }

    let edited = std::fs::read_to_string(&temp_path)?;
    let cleanup = std::fs::remove_file(&temp_path);
    debug_log::log_kv(
        "edit.temp_file",
        &[
            debug_log::field("phase", "cleanup"),
            debug_log::field(
                "result",
                if cleanup.is_ok() { "ok" } else { "error" },
            ),
            debug_log::field("path", temp_path.display()),
        ],
    );

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
            debug_log::log_kv(
                "edit.save",
                &[
                    debug_log::field("kind", "character"),
                    debug_log::field("slug", &slug),
                    debug_log::field("new_slug", &new_slug),
                    debug_log::field("renamed", new_slug != slug),
                    debug_log::field("operation", operation),
                    debug_log::field("bytes", edited.len()),
                    debug_log::field("result", "ok"),
                ],
            );
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
            debug_log::log_kv(
                "edit.save",
                &[
                    debug_log::field("kind", "worldbook"),
                    debug_log::field("slug", &slug),
                    debug_log::field("new_slug", &new_slug),
                    debug_log::field("renamed", new_slug != slug),
                    debug_log::field("operation", operation),
                    debug_log::field("bytes", edited.len()),
                    debug_log::field("result", "ok"),
                ],
            );
            eprintln!("Saved worldbook: {}", wb.name);
        }
        _ => unreachable!(),
    }

    Ok(())
}
