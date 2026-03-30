use crate::crypto::DerivedKey;

pub struct MigrationResult {
    pub changed_count: usize,
    pub warnings: Vec<String>,
}

pub fn migrate_config_path() -> MigrationResult {
    crate::config::migrate_config();
    MigrationResult {
        changed_count: 0,
        warnings: Vec::new(),
    }
}

pub fn migrate_system_prompts_from_config(key: Option<&DerivedKey>) -> MigrationResult {
    let dir = crate::config::system_prompts_dir();
    crate::system_prompt::migrate_from_config(&dir, key);
    MigrationResult {
        changed_count: 0,
        warnings: Vec::new(),
    }
}

pub fn migrate_personas_from_config(key: Option<&DerivedKey>) -> MigrationResult {
    let dir = crate::config::personas_dir();
    let cfg = crate::config::load();

    if cfg.user_name.is_none() && cfg.user_persona.is_none() {
        return MigrationResult {
            changed_count: 0,
            warnings: Vec::new(),
        };
    }

    let name = cfg.user_name.clone().unwrap_or_default();
    let file_name = if name.is_empty() {
        "default".to_owned()
    } else {
        name.clone()
    };

    let existing_path = crate::persona::resolve_persona_path(&dir, &file_name);
    let mut changed = 0;

    if !existing_path.exists() {
        let persona = crate::persona::PersonaFile {
            name: if name.is_empty() {
                file_name
            } else {
                name
            },
            persona: cfg.user_persona.clone().unwrap_or_default(),
        };
        if crate::persona::save_persona(&persona, &dir, key).is_ok() {
            changed = 1;
        }
    }

    if let Err(e) = crate::config::save(&cfg) {
        eprintln!("Warning: failed to save config during persona migration: {e}");
    }

    MigrationResult {
        changed_count: changed,
        warnings: Vec::new(),
    }
}

pub fn migrate_worldbook_normalization(key: Option<&DerivedKey>) -> MigrationResult {
    let dir = crate::config::worldinfo_dir();
    let report = crate::worldinfo::normalize_worldbooks(&dir, key);
    MigrationResult {
        changed_count: report.rewritten_count,
        warnings: report.warnings,
    }
}

pub fn migrate_encrypt_plaintext_cards(key: &DerivedKey) -> MigrationResult {
    let dir = crate::config::characters_dir();
    let report = crate::character::encrypt_plaintext_cards(&dir, key);
    MigrationResult {
        changed_count: report.encrypted_count,
        warnings: report.warnings,
    }
}

pub fn migrate_encrypt_plaintext_prompts(key: &DerivedKey) -> MigrationResult {
    let dir = crate::config::system_prompts_dir();
    let warnings = crate::system_prompt::encrypt_plaintext_prompts(&dir, key);
    MigrationResult {
        changed_count: warnings.len(),
        warnings,
    }
}

pub fn migrate_encrypt_plaintext_personas(key: &DerivedKey) -> MigrationResult {
    let dir = crate::config::personas_dir();
    let warnings = crate::persona::encrypt_plaintext_personas(&dir, key);
    MigrationResult {
        changed_count: warnings.len(),
        warnings,
    }
}

pub fn migrate_index_rename() -> MigrationResult {
    let old_path = crate::config::data_dir().join("index.json");
    let new_path = crate::config::index_path();
    if !new_path.exists() && old_path.exists() {
        if let Err(e) = std::fs::rename(&old_path, &new_path) {
            return MigrationResult {
                changed_count: 0,
                warnings: vec![format!("failed to rename index.json to index.meta: {e}")],
            };
        }
        return MigrationResult {
            changed_count: 1,
            warnings: Vec::new(),
        };
    }
    MigrationResult {
        changed_count: 0,
        warnings: Vec::new(),
    }
}

pub fn migrate_encrypt_plaintext_index(key: &DerivedKey) -> MigrationResult {
    let path = crate::config::index_path();
    if !path.exists() {
        return MigrationResult {
            changed_count: 0,
            warnings: Vec::new(),
        };
    }
    let raw = match std::fs::read(&path) {
        Ok(data) => data,
        Err(e) => {
            return MigrationResult {
                changed_count: 0,
                warnings: vec![format!("failed to read {}: {e}", path.display())],
            };
        }
    };
    if crate::crypto::is_encrypted(&raw) {
        return MigrationResult {
            changed_count: 0,
            warnings: Vec::new(),
        };
    }
    if let Err(e) = crate::crypto::encrypt_and_write(&path, &raw, Some(key)) {
        return MigrationResult {
            changed_count: 0,
            warnings: vec![format!("failed to encrypt {}: {e}", path.display())],
        };
    }
    MigrationResult {
        changed_count: 1,
        warnings: Vec::new(),
    }
}
