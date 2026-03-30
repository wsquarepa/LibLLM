use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::crypto::DerivedKey;

const EXT_ENCRYPTED: &str = "prompt";
const EXT_PLAINTEXT: &str = "json";

pub const BUILTIN_ASSISTANT: &str = "assistant";
pub const BUILTIN_ROLEPLAY: &str = "roleplay";

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemPromptFile {
    pub name: String,
    pub content: String,
}

pub struct SystemPromptEntry {
    pub name: String,
}

pub fn resolve_prompt_path(dir: &Path, name: &str) -> PathBuf {
    crate::crypto::resolve_encrypted_path(dir, name, EXT_ENCRYPTED)
}

pub fn load_prompt(path: &Path, key: Option<&DerivedKey>) -> Result<SystemPromptFile> {
    let contents = crate::crypto::read_and_decrypt(path, key)?;
    serde_json::from_str(&contents).context("failed to parse system prompt JSON")
}

pub fn save_prompt(prompt: &SystemPromptFile, dir: &Path, key: Option<&DerivedKey>) -> Result<PathBuf> {
    let ext = crate::crypto::encrypted_extension(key, EXT_ENCRYPTED);
    let safe_name: String = prompt.name.replace(['/', '\\'], "_");
    let safe_name = safe_name.trim_matches('.');
    anyhow::ensure!(!safe_name.is_empty(), "prompt name is empty after sanitization");
    let path = dir.join(format!("{safe_name}.{ext}"));
    anyhow::ensure!(path.starts_with(dir), "prompt path escapes target directory");
    save_prompt_to(prompt, &path, key)?;
    Ok(path)
}

fn save_prompt_to(prompt: &SystemPromptFile, path: &Path, key: Option<&DerivedKey>) -> Result<()> {
    let json = serde_json::to_string_pretty(prompt).context("failed to serialize system prompt")?;
    crate::crypto::encrypt_and_write(path, json.as_bytes(), key)
}

pub fn load_prompt_content(dir: &Path, name: &str, key: Option<&DerivedKey>) -> Option<String> {
    let path = resolve_prompt_path(dir, name);
    load_prompt(&path, key).ok().map(|p| p.content)
}

pub fn list_prompts(dir: &Path, _key: Option<&DerivedKey>) -> Vec<SystemPromptEntry> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut builtins: Vec<SystemPromptEntry> = Vec::new();
    let mut custom: Vec<SystemPromptEntry> = Vec::new();

    for path in entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == EXT_ENCRYPTED || ext == EXT_PLAINTEXT)
        })
    {
        let name = match path.file_stem() {
            Some(stem) => stem.to_string_lossy().to_string(),
            None => continue,
        };

        if name == BUILTIN_ASSISTANT || name == BUILTIN_ROLEPLAY {
            builtins.push(SystemPromptEntry { name });
        } else {
            custom.push(SystemPromptEntry { name });
        }
    }

    custom.sort_by(|a, b| a.name.cmp(&b.name));

    let mut result: Vec<SystemPromptEntry> = Vec::new();

    let has_assistant = builtins.iter().any(|e| e.name == BUILTIN_ASSISTANT);
    let has_roleplay = builtins.iter().any(|e| e.name == BUILTIN_ROLEPLAY);
    if has_assistant {
        result.push(SystemPromptEntry {
            name: BUILTIN_ASSISTANT.to_owned(),
        });
    }
    if has_roleplay {
        result.push(SystemPromptEntry {
            name: BUILTIN_ROLEPLAY.to_owned(),
        });
    }

    result.extend(custom);
    result
}

pub fn ensure_builtin_prompts(dir: &Path, key: Option<&DerivedKey>) {
    for builtin_name in [BUILTIN_ASSISTANT, BUILTIN_ROLEPLAY] {
        let path = resolve_prompt_path(dir, builtin_name);
        if !path.exists() {
            let prompt = SystemPromptFile {
                name: builtin_name.to_owned(),
                content: String::new(),
            };
            if let Err(e) = save_prompt(&prompt, dir, key) {
                eprintln!("Warning: failed to create {builtin_name} system prompt: {e}");
            }
        }
    }
}

pub fn migrate_from_config(dir: &Path, key: Option<&DerivedKey>) {
    let cfg = crate::config::load();

    let mut config_changed = false;
    let mut new_cfg = cfg;

    if let Some(ref content) = new_cfg.system_prompt {
        if !content.is_empty() && !is_name_reference(dir, content) {
            let prompt = SystemPromptFile {
                name: BUILTIN_ASSISTANT.to_owned(),
                content: content.clone(),
            };
            if save_prompt(&prompt, dir, key).is_ok() {
                new_cfg.system_prompt = Some(BUILTIN_ASSISTANT.to_owned());
                config_changed = true;
            }
        }
    }

    if let Some(ref content) = new_cfg.roleplay_system_prompt {
        if !content.is_empty() && !is_name_reference(dir, content) {
            let prompt = SystemPromptFile {
                name: BUILTIN_ROLEPLAY.to_owned(),
                content: content.clone(),
            };
            if save_prompt(&prompt, dir, key).is_ok() {
                new_cfg.roleplay_system_prompt = Some(BUILTIN_ROLEPLAY.to_owned());
                config_changed = true;
            }
        }
    }

    if config_changed {
        let _ = crate::config::save(&new_cfg);
    }
}

fn is_name_reference(dir: &Path, value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.contains('\n') || trimmed.contains(' ') || trimmed.len() > 64 {
        return false;
    }
    let path = resolve_prompt_path(dir, trimmed);
    path.exists()
}

pub fn encrypt_plaintext_prompts(dir: &Path, key: &DerivedKey) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warnings.push(format!("failed to read system prompts dir: {e}"));
            return warnings;
        }
    };

    let json_paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();

    for path in json_paths {
        let raw = match std::fs::read(&path) {
            Ok(r) => r,
            Err(e) => {
                warnings.push(format!("skipped {}: {e}", path.display()));
                continue;
            }
        };
        if crate::crypto::is_encrypted(&raw) {
            continue;
        }

        let stem = match path.file_stem() {
            Some(s) => s.to_string_lossy().to_string(),
            None => continue,
        };

        let encrypted_path = dir.join(format!("{stem}.{EXT_ENCRYPTED}"));
        if let Err(e) = crate::crypto::encrypt_and_write(&encrypted_path, &raw, Some(key)) {
            warnings.push(format!("failed to encrypt {}: {e}", path.display()));
            continue;
        }

        if let Err(e) = std::fs::remove_file(&path) {
            warnings.push(format!("encrypted but failed to remove plaintext {}: {e}", path.display()));
        }
    }

    warnings
}
