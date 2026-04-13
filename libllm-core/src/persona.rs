use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::crypto::DerivedKey;

const EXT_ENCRYPTED: &str = "persona";
const EXT_PLAINTEXT: &str = "json";

#[derive(Debug, Serialize, Deserialize)]
pub struct PersonaFile {
    pub name: String,
    pub persona: String,
}

pub struct PersonaEntry {
    pub name: String,
}

fn sanitize_persona_slug(name: &str) -> Option<String> {
    let safe_name: String = name.replace(['/', '\\'], "_");
    let safe_name = safe_name.trim_matches('.');
    if safe_name.is_empty() {
        None
    } else {
        Some(safe_name.to_owned())
    }
}

pub fn resolve_persona_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let slug = sanitize_persona_slug(name)?;
    Some(crate::crypto::resolve_encrypted_path(
        dir,
        &slug,
        EXT_ENCRYPTED,
    ))
}

pub fn load_persona(path: &Path, key: Option<&DerivedKey>) -> Result<PersonaFile> {
    let contents = crate::crypto::read_and_decrypt(path, key)?;
    serde_json::from_str(&contents).context("failed to parse persona JSON")
}

pub fn save_persona(
    persona: &PersonaFile,
    dir: &Path,
    key: Option<&DerivedKey>,
) -> Result<PathBuf> {
    let ext = crate::crypto::encrypted_extension(key, EXT_ENCRYPTED);
    let safe_name = sanitize_persona_slug(&persona.name).unwrap_or_default();
    anyhow::ensure!(
        !safe_name.is_empty(),
        "persona name is empty after sanitization"
    );
    let path = dir.join(format!("{safe_name}.{ext}"));
    anyhow::ensure!(
        path.starts_with(dir),
        "persona path escapes target directory"
    );
    let json = serde_json::to_string_pretty(persona).context("failed to serialize persona")?;
    crate::crypto::encrypt_and_write(&path, json.as_bytes(), key)?;
    Ok(path)
}

pub fn load_persona_by_name(
    dir: &Path,
    name: &str,
    key: Option<&DerivedKey>,
) -> Option<PersonaFile> {
    let path = resolve_persona_path(dir, name)?;
    load_persona(&path, key).ok()
}

pub fn list_personas(dir: &Path, _key: Option<&DerivedKey>) -> Vec<PersonaEntry> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut result: Vec<PersonaEntry> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == EXT_ENCRYPTED || ext == EXT_PLAINTEXT)
        })
        .filter_map(|path| {
            path.file_stem().map(|stem| PersonaEntry {
                name: stem.to_string_lossy().to_string(),
            })
        })
        .collect();

    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

pub fn encrypt_plaintext_personas(dir: &Path, key: &DerivedKey) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warnings.push(format!("failed to read personas dir: {e}"));
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
            warnings.push(format!(
                "encrypted but failed to remove plaintext {}: {e}",
                path.display()
            ));
        }
    }

    warnings
}
