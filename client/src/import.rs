//! File import for characters, worldbooks, personas, and system prompts.

use anyhow::{Context, Result};
use libllm::character;
use libllm::db::Database;

pub enum ImportType {
    Character,
    Worldbook,
    Persona,
    SystemPrompt,
}

pub fn parse_import_kind(kind: &str) -> Result<ImportType> {
    match kind {
        "character" | "char" => Ok(ImportType::Character),
        "worldbook" | "wb" | "book" => Ok(ImportType::Worldbook),
        "persona" => Ok(ImportType::Persona),
        "prompt" | "system-prompt" => Ok(ImportType::SystemPrompt),
        _ => anyhow::bail!(
            "Unknown import type: {kind}. \
             Use: character, char, worldbook, wb, book, persona, prompt, system-prompt"
        ),
    }
}

pub fn detect_import_type(path: &std::path::Path, kind: Option<&str>) -> Result<ImportType> {
    if let Some(kind) = kind {
        libllm::debug_log::log_kv(
            "import.detect_type",
            &[
                libllm::debug_log::field("path", path.display()),
                libllm::debug_log::field("mode", "explicit"),
                libllm::debug_log::field("kind", kind),
            ],
        );
        return parse_import_kind(kind);
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "png" => Ok(ImportType::Character),
        "txt" => anyhow::bail!(
            "{}: .txt files are ambiguous. Use --type persona or --type prompt",
            path.display()
        ),
        "json" => {
            let contents = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            libllm::debug_log::log_kv(
                "import.detect_type",
                &[
                    libllm::debug_log::field("path", path.display()),
                    libllm::debug_log::field("mode", "auto"),
                    libllm::debug_log::field("extension", "json"),
                    libllm::debug_log::field("bytes", contents.len()),
                ],
            );

            if character::parse_card_json(&contents).is_ok() {
                return Ok(ImportType::Character);
            }

            let fallback_name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if libllm::worldinfo::parse_worldbook_json(&contents, &fallback_name).is_ok() {
                return Ok(ImportType::Worldbook);
            }

            anyhow::bail!(
                "{}: JSON file does not match character or worldbook format. \
                 Use --type to specify the content type.",
                path.display()
            )
        }
        _ => anyhow::bail!(
            "{}: unsupported file extension '.{ext}'. Supported: .json, .png, .txt",
            path.display()
        ),
    }
}

pub fn import_single_file(
    path: &std::path::Path,
    import_type: &ImportType,
    db: &Database,
) -> Result<String> {
    match import_type {
        ImportType::Character => {
            let card = character::import_card(path)?;
            let slug = character::slugify(&card.name);
            db.insert_character(&slug, &card)?;
            Ok(format!("Imported character: \"{}\" ({})", card.name, slug))
        }
        ImportType::Worldbook => {
            let contents = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let fallback_name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let wb = libllm::worldinfo::parse_worldbook_json(&contents, &fallback_name)?;
            let slug = character::slugify(&wb.name);
            db.insert_worldbook(&slug, &wb)?;
            Ok(format!("Imported worldbook: \"{}\" ({})", wb.name, slug))
        }
        ImportType::Persona => {
            let contents = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(sanitize_name)
                .flatten()
                .ok_or_else(|| {
                    anyhow::anyhow!("{}: invalid filename for persona name", path.display())
                })?;
            let slug = character::slugify(&name);
            let persona = libllm::persona::PersonaFile {
                name: name.clone(),
                persona: contents,
            };
            db.insert_persona(&slug, &persona)?;
            Ok(format!("Imported persona: \"{}\" ({})", name, slug))
        }
        ImportType::SystemPrompt => {
            let contents = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(sanitize_name)
                .flatten()
                .ok_or_else(|| {
                    anyhow::anyhow!("{}: invalid filename for prompt name", path.display())
                })?;
            let slug = character::slugify(&name);
            let prompt = libllm::system_prompt::SystemPromptFile {
                name: name.clone(),
                content: contents,
            };
            db.insert_prompt(&slug, &prompt, false)?;
            Ok(format!("Imported system prompt: \"{}\" ({})", name, slug))
        }
    }
}

pub fn sanitize_name(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_import_kind_valid() {
        assert!(matches!(
            parse_import_kind("character").unwrap(),
            ImportType::Character
        ));
        assert!(matches!(
            parse_import_kind("char").unwrap(),
            ImportType::Character
        ));
        assert!(matches!(
            parse_import_kind("worldbook").unwrap(),
            ImportType::Worldbook
        ));
        assert!(matches!(
            parse_import_kind("wb").unwrap(),
            ImportType::Worldbook
        ));
        assert!(matches!(
            parse_import_kind("book").unwrap(),
            ImportType::Worldbook
        ));
        assert!(matches!(
            parse_import_kind("persona").unwrap(),
            ImportType::Persona
        ));
        assert!(matches!(
            parse_import_kind("prompt").unwrap(),
            ImportType::SystemPrompt
        ));
        assert!(matches!(
            parse_import_kind("system-prompt").unwrap(),
            ImportType::SystemPrompt
        ));
    }

    #[test]
    fn parse_import_kind_invalid() {
        assert!(parse_import_kind("invalid").is_err());
        assert!(parse_import_kind("").is_err());
    }

    #[test]
    fn sanitize_name_normal() {
        assert_eq!(
            sanitize_name("hello-world_1"),
            Some("hello-world_1".to_string())
        );
    }

    #[test]
    fn sanitize_name_empty_after_strip() {
        assert!(sanitize_name("!@#$%").is_none());
    }

    #[test]
    fn sanitize_name_trims_whitespace() {
        let result = sanitize_name("  hello  ");
        assert_eq!(result, Some("hello".to_string()));
    }
}

pub fn handle_import_command(
    files: &[std::path::PathBuf],
    kind: Option<&str>,
    db: &Database,
) -> Result<()> {
    libllm::debug_log::log_kv(
        "import.run",
        &[
            libllm::debug_log::field("file_count", files.len()),
            libllm::debug_log::field("kind", kind.unwrap_or("auto")),
        ],
    );

    if files.is_empty() {
        anyhow::bail!("No files specified. Usage: libllm import <file>...");
    }

    let mut had_errors = false;

    for file in files {
        if !file.exists() {
            libllm::debug_log::log_kv(
                "import.file",
                &[
                    libllm::debug_log::field("path", file.display()),
                    libllm::debug_log::field("result", "error"),
                    libllm::debug_log::field("reason", "not_found"),
                ],
            );
            eprintln!("Error: {}: file not found", file.display());
            had_errors = true;
            continue;
        }
        if !file.is_file() {
            libllm::debug_log::log_kv(
                "import.file",
                &[
                    libllm::debug_log::field("path", file.display()),
                    libllm::debug_log::field("result", "error"),
                    libllm::debug_log::field("reason", "not_regular_file"),
                ],
            );
            eprintln!("Error: {}: not a regular file", file.display());
            had_errors = true;
            continue;
        }

        match detect_import_type(file, kind) {
            Ok(import_type) => match import_single_file(file, &import_type, db) {
                Ok(msg) => {
                    libllm::debug_log::log_kv(
                        "import.file",
                        &[
                            libllm::debug_log::field("path", file.display()),
                            libllm::debug_log::field("result", "ok"),
                        ],
                    );
                    eprintln!("{msg}")
                }
                Err(e) => {
                    libllm::debug_log::log_kv(
                        "import.file",
                        &[
                            libllm::debug_log::field("path", file.display()),
                            libllm::debug_log::field("result", "error"),
                            libllm::debug_log::field("reason", "import_failed"),
                            libllm::debug_log::field("error", &e),
                        ],
                    );
                    eprintln!("Error: {}: {e}", file.display());
                    had_errors = true;
                }
            },
            Err(e) => {
                libllm::debug_log::log_kv(
                    "import.file",
                    &[
                        libllm::debug_log::field("path", file.display()),
                        libllm::debug_log::field("result", "error"),
                        libllm::debug_log::field("reason", "type_detection_failed"),
                        libllm::debug_log::field("error", &e),
                    ],
                );
                eprintln!("Error: {e}");
                had_errors = true;
            }
        }
    }

    if had_errors {
        anyhow::bail!("Some imports failed.");
    }
    Ok(())
}
