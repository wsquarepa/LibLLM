use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result, bail};
use libllm_core::character::{CharacterCard, extract_png_card, parse_card_json};
use libllm_core::persona::PersonaFile;
use libllm_core::session::Session;
use libllm_core::system_prompt::SystemPromptFile;
use libllm_core::worldinfo::WorldBook;

const MAGIC: &[u8; 4] = b"LLMS";
const VERSION: u8 = 0x01;
const HEADER_LEN: usize = 4 + 1 + 12;

fn is_encrypted(data: &[u8]) -> bool {
    data.len() >= HEADER_LEN && data[..4] == *MAGIC
}

fn decrypt_bytes(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    if data.len() < HEADER_LEN {
        bail!("file too short");
    }
    if &data[..4] != MAGIC {
        bail!("not an encrypted file");
    }
    if data[4] != VERSION {
        bail!("unsupported version");
    }
    let nonce = Nonce::from_slice(&data[5..17]);
    let cipher =
        Aes256Gcm::new_from_slice(key).context("cipher init")?;
    cipher
        .decrypt(nonce, &data[17..])
        .map_err(|_| anyhow::anyhow!("decryption failed"))
}

fn read_and_decode(path: &Path, key_bytes: Option<&[u8; 32]>) -> Result<String> {
    let raw =
        std::fs::read(path).with_context(|| format!("failed to read file: {}", path.display()))?;
    if is_encrypted(&raw) {
        let key = key_bytes.ok_or_else(|| {
            anyhow::anyhow!(
                "file is encrypted but no key provided: {}",
                path.display()
            )
        })?;
        let decrypted = decrypt_bytes(&raw, key)?;
        String::from_utf8(decrypted).context("decrypted content is not valid UTF-8")
    } else {
        String::from_utf8(raw).context("file content is not valid UTF-8")
    }
}

fn dir_has_files(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .ok()
        .is_some_and(|mut entries| entries.next().is_some())
}

pub fn has_legacy_files(data_dir: &Path) -> bool {
    let sessions = data_dir.join("sessions");
    let characters = data_dir.join("characters");
    let worldinfo = data_dir.join("worldinfo");
    let system = data_dir.join("system");
    let personas = data_dir.join("personas");
    dir_has_files(&sessions)
        || dir_has_files(&characters)
        || dir_has_files(&worldinfo)
        || dir_has_files(&system)
        || dir_has_files(&personas)
}

fn collect_files_with_extensions(dir: &Path, extensions: &[&str]) -> Vec<std::path::PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| extensions.contains(&ext))
        })
        .collect()
}

fn slug_from_path(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

pub fn read_sessions(
    dir: &Path,
    key_bytes: Option<&[u8; 32]>,
) -> Result<Vec<(String, Session)>> {
    let files = collect_files_with_extensions(dir, &["session"]);
    let mut results: Vec<(String, Session)> = Vec::new();
    for path in files {
        let slug = slug_from_path(&path);
        match read_and_decode(&path, key_bytes) {
            Ok(json) => match serde_json::from_str::<Session>(&json) {
                Ok(session) => results.push((slug, session)),
                Err(e) => eprintln!("warning: failed to parse session {}: {e}", path.display()),
            },
            Err(e) => eprintln!("warning: failed to read session {}: {e}", path.display()),
        }
    }
    Ok(results)
}

pub fn read_characters(
    dir: &Path,
    key_bytes: Option<&[u8; 32]>,
) -> Result<Vec<(String, CharacterCard)>> {
    let files = collect_files_with_extensions(dir, &["character", "json", "png"]);
    let mut results: Vec<(String, CharacterCard)> = Vec::new();
    for path in files {
        let slug = slug_from_path(&path);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        match ext {
            "png" => {
                let bytes = match std::fs::read(&path) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!(
                            "warning: failed to read PNG character {}: {e}",
                            path.display()
                        );
                        continue;
                    }
                };
                match extract_png_card(&bytes).and_then(|json| parse_card_json(&json)) {
                    Ok(card) => results.push((slug, card)),
                    Err(e) => eprintln!(
                        "warning: failed to parse PNG character {}: {e}",
                        path.display()
                    ),
                }
            }
            _ => match read_and_decode(&path, key_bytes) {
                Ok(json) => match parse_card_json(&json) {
                    Ok(card) => results.push((slug, card)),
                    Err(e) => eprintln!(
                        "warning: failed to parse character {}: {e}",
                        path.display()
                    ),
                },
                Err(e) => {
                    eprintln!("warning: failed to read character {}: {e}", path.display())
                }
            },
        }
    }
    Ok(results)
}

pub fn read_worldbooks(
    dir: &Path,
    key_bytes: Option<&[u8; 32]>,
) -> Result<Vec<(String, WorldBook)>> {
    let files = collect_files_with_extensions(dir, &["worldbook", "json"]);
    let mut results: Vec<(String, WorldBook)> = Vec::new();
    for path in files {
        let slug = slug_from_path(&path);
        match read_and_decode(&path, key_bytes) {
            Ok(json) => match serde_json::from_str::<WorldBook>(&json) {
                Ok(wb) => results.push((slug, wb)),
                Err(_) => {
                    match libllm_core::worldinfo::load_worldbook(&path, None) {
                        Ok(wb) => results.push((slug, wb)),
                        Err(e) => eprintln!(
                            "warning: failed to parse worldbook {}: {e}",
                            path.display()
                        ),
                    }
                }
            },
            Err(e) => eprintln!("warning: failed to read worldbook {}: {e}", path.display()),
        }
    }
    Ok(results)
}

pub fn read_personas(
    dir: &Path,
    key_bytes: Option<&[u8; 32]>,
) -> Result<Vec<(String, PersonaFile)>> {
    let files = collect_files_with_extensions(dir, &["persona", "json"]);
    let mut results: Vec<(String, PersonaFile)> = Vec::new();
    for path in files {
        let slug = slug_from_path(&path);
        match read_and_decode(&path, key_bytes) {
            Ok(json) => match serde_json::from_str::<PersonaFile>(&json) {
                Ok(persona) => results.push((slug, persona)),
                Err(e) => {
                    eprintln!("warning: failed to parse persona {}: {e}", path.display())
                }
            },
            Err(e) => eprintln!("warning: failed to read persona {}: {e}", path.display()),
        }
    }
    Ok(results)
}

pub fn read_prompts(
    dir: &Path,
    key_bytes: Option<&[u8; 32]>,
) -> Result<Vec<(String, SystemPromptFile)>> {
    let files = collect_files_with_extensions(dir, &["prompt", "json"]);
    let mut results: Vec<(String, SystemPromptFile)> = Vec::new();
    for path in files {
        let slug = slug_from_path(&path);
        match read_and_decode(&path, key_bytes) {
            Ok(json) => match serde_json::from_str::<SystemPromptFile>(&json) {
                Ok(prompt) => results.push((slug, prompt)),
                Err(e) => eprintln!(
                    "warning: failed to parse system prompt {}: {e}",
                    path.display()
                ),
            },
            Err(e) => {
                eprintln!(
                    "warning: failed to read system prompt {}: {e}",
                    path.display()
                )
            }
        }
    }
    Ok(results)
}
