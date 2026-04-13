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

    let existing_path = match crate::persona::resolve_persona_path(&dir, &file_name) {
        Some(path) => path,
        None => {
            return MigrationResult {
                changed_count: 0,
                warnings: vec!["skipped persona migration: invalid persona name".to_owned()],
            };
        }
    };
    let mut changed = 0;

    if !existing_path.exists() {
        let persona = crate::persona::PersonaFile {
            name: if name.is_empty() { file_name } else { name },
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
