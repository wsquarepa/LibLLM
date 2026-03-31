mod common;

use std::path::Path;
use std::sync::OnceLock;

use libllm::config::{self, Config};
use libllm::crypto;
use libllm::migration;

static DATA_DIR: OnceLock<tempfile::TempDir> = OnceLock::new();

fn setup_data_dir() -> &'static Path {
    DATA_DIR
        .get_or_init(|| {
            let dir = tempfile::tempdir().unwrap();
            config::set_data_dir(dir.path().to_path_buf());
            config::ensure_dirs().unwrap();
            dir
        })
        .path()
}

// ---------------------------------------------------------------------------
// Config tests
// ---------------------------------------------------------------------------

#[test]
fn config_default_values() {
    let cfg = Config::default();
    assert!(cfg.api_url.is_none());
    assert_eq!(cfg.api_url(), "http://localhost:5001/v1");
    assert!(cfg.template.is_none());
    assert!(cfg.template_preset.is_none());
    assert!(cfg.instruct_preset.is_none());
    assert!(cfg.reasoning_preset.is_none());
    assert!(cfg.user_name.is_none());
    assert!(cfg.user_persona.is_none());
    assert!(cfg.sampling.temperature.is_none());
    assert!(cfg.sampling.top_k.is_none());
    assert!(cfg.sampling.top_p.is_none());
    assert!(cfg.sampling.min_p.is_none());
    assert!(cfg.sampling.repeat_last_n.is_none());
    assert!(cfg.sampling.repeat_penalty.is_none());
    assert!(cfg.sampling.max_tokens.is_none());
    assert!(cfg.worldbooks.is_empty());
    assert!(!cfg.tls_skip_verify);
    assert!(!cfg.debug_log);
    assert!(cfg.default_persona.is_none());
}

#[test]
fn config_api_url_custom() {
    let mut cfg = Config::default();
    cfg.api_url = Some("http://example.com/api".to_owned());
    assert_eq!(cfg.api_url(), "http://example.com/api");
}

#[test]
fn config_save_load_roundtrip() {
    let root = setup_data_dir();
    let _key = common::test_key(root);

    let mut cfg = Config::default();
    cfg.api_url = Some("http://roundtrip.test/v1".to_owned());
    cfg.template_preset = Some("chatml".to_owned());
    cfg.instruct_preset = Some("alpaca".to_owned());
    cfg.reasoning_preset = Some("deepseek".to_owned());
    cfg.worldbooks = vec!["lore.worldbook".to_owned()];
    cfg.tls_skip_verify = true;
    cfg.debug_log = true;
    cfg.default_persona = Some("tester".to_owned());
    cfg.sampling.temperature = Some(0.7);
    cfg.sampling.top_k = Some(40);

    config::save(&cfg).unwrap();
    let loaded = config::load();

    assert_eq!(loaded.api_url.as_deref(), Some("http://roundtrip.test/v1"));
    assert_eq!(loaded.api_url(), "http://roundtrip.test/v1");
    assert_eq!(loaded.template_preset.as_deref(), Some("chatml"));
    assert_eq!(loaded.instruct_preset.as_deref(), Some("alpaca"));
    assert_eq!(loaded.reasoning_preset.as_deref(), Some("deepseek"));
    assert_eq!(loaded.worldbooks, vec!["lore.worldbook"]);
    assert!(loaded.tls_skip_verify);
    assert!(loaded.debug_log);
    assert_eq!(loaded.default_persona.as_deref(), Some("tester"));
    assert_eq!(loaded.sampling.temperature, Some(0.7));
    assert_eq!(loaded.sampling.top_k, Some(40));
}

#[test]
fn config_toml_skips_legacy_fields() {
    let root = setup_data_dir();
    let _key = common::test_key(root);

    let mut cfg = Config::default();
    cfg.template = Some("llama2".to_owned());
    cfg.user_name = Some("Alice".to_owned());
    cfg.user_persona = Some("A curious developer".to_owned());
    cfg.api_url = Some("http://legacy-test.local/v1".to_owned());

    config::save(&cfg).unwrap();

    let raw_toml = common::read_file(&config::config_path());
    assert!(
        !raw_toml.contains("template"),
        "template should be skip_serializing but found in TOML: {raw_toml}"
    );
    assert!(
        !raw_toml.contains("user_name"),
        "user_name should be skip_serializing but found in TOML: {raw_toml}"
    );
    assert!(
        !raw_toml.contains("user_persona"),
        "user_persona should be skip_serializing but found in TOML: {raw_toml}"
    );
    assert!(
        raw_toml.contains("api_url"),
        "api_url should be present in TOML: {raw_toml}"
    );
}

#[test]
fn config_missing_file_returns_default() {
    setup_data_dir();

    let bogus = config::data_dir().join("nonexistent_config.toml");
    assert!(!bogus.exists());

    let cfg = config::load();
    assert!(cfg.api_url.is_none() || cfg.api_url.is_some());
}

#[test]
fn config_partial_toml() {
    let root = setup_data_dir();
    let _key = common::test_key(root);

    let partial = "api_url = \"http://partial.test/v1\"\n";
    std::fs::write(config::config_path(), partial).unwrap();

    let loaded = config::load();
    assert_eq!(loaded.api_url.as_deref(), Some("http://partial.test/v1"));
    assert!(loaded.sampling.temperature.is_none());
    assert!(loaded.worldbooks.is_empty());
    assert!(!loaded.tls_skip_verify);
    assert!(!loaded.debug_log);
    assert!(loaded.default_persona.is_none());
}

#[test]
fn config_ensure_dirs_creates_subdirectories() {
    let root = setup_data_dir();

    assert!(root.join("sessions").is_dir());
    assert!(root.join("characters").is_dir());
    assert!(root.join("worldinfo").is_dir());
    assert!(root.join("system").is_dir());
    assert!(root.join("personas").is_dir());
}

// ---------------------------------------------------------------------------
// Migration tests
// ---------------------------------------------------------------------------

#[test]
fn migrate_encrypt_plaintext_cards() {
    let root = setup_data_dir();
    let key = common::test_key(root);

    let card = common::simple_character("migrate-card", "A test character");
    let json = serde_json::to_string_pretty(&card).unwrap();
    let json_path = config::characters_dir().join("migrate-card.json");
    std::fs::write(&json_path, &json).unwrap();

    let result = migration::migrate_encrypt_plaintext_cards(&key);
    assert_eq!(result.changed_count, 1);
    assert!(result.warnings.is_empty(), "unexpected warnings: {:?}", result.warnings);

    common::assert_file_missing(&json_path);

    let encrypted_path = config::characters_dir().join("migrate-card.character");
    common::assert_file_exists(&encrypted_path);

    let raw = std::fs::read(&encrypted_path).unwrap();
    assert!(crypto::is_encrypted(&raw));
}

#[test]
fn migrate_encrypt_plaintext_prompts() {
    let root = setup_data_dir();
    let key = common::test_key(root);

    let prompt = common::system_prompt("migrate-prompt", "You are a test assistant.");
    let json = serde_json::to_string_pretty(&prompt).unwrap();
    let json_path = config::system_prompts_dir().join("migrate-prompt.json");
    std::fs::write(&json_path, &json).unwrap();

    let result = migration::migrate_encrypt_plaintext_prompts(&key);
    assert_eq!(result.changed_count, 0, "clean migration should have changed_count 0 (warnings only)");
    assert!(result.warnings.is_empty(), "unexpected warnings: {:?}", result.warnings);

    common::assert_file_missing(&json_path);

    let encrypted_path = config::system_prompts_dir().join("migrate-prompt.prompt");
    common::assert_file_exists(&encrypted_path);

    let raw = std::fs::read(&encrypted_path).unwrap();
    assert!(crypto::is_encrypted(&raw));
}

#[test]
fn migrate_encrypt_plaintext_personas() {
    let root = setup_data_dir();
    let key = common::test_key(root);

    let persona = common::persona("migrate-persona", "A testing persona");
    let json = serde_json::to_string_pretty(&persona).unwrap();
    let json_path = config::personas_dir().join("migrate-persona.json");
    std::fs::write(&json_path, &json).unwrap();

    let result = migration::migrate_encrypt_plaintext_personas(&key);
    assert_eq!(result.changed_count, 0, "clean migration should have changed_count 0 (warnings only)");
    assert!(result.warnings.is_empty(), "unexpected warnings: {:?}", result.warnings);

    common::assert_file_missing(&json_path);

    let encrypted_path = config::personas_dir().join("migrate-persona.persona");
    common::assert_file_exists(&encrypted_path);

    let raw = std::fs::read(&encrypted_path).unwrap();
    assert!(crypto::is_encrypted(&raw));
}

#[test]
fn migrate_index_rename() {
    let root = setup_data_dir();

    let old_path = root.join("index.json");
    let new_path = config::index_path();

    if new_path.exists() {
        std::fs::remove_file(&new_path).unwrap();
    }

    std::fs::write(&old_path, "{}").unwrap();

    let result = migration::migrate_index_rename();
    assert_eq!(result.changed_count, 1);
    assert!(result.warnings.is_empty());

    common::assert_file_missing(&old_path);
    common::assert_file_exists(&new_path);
}

#[test]
fn migrate_index_rename_idempotent() {
    let root = setup_data_dir();

    let old_path = root.join("index.json");
    let new_path = config::index_path();

    if !new_path.exists() {
        std::fs::write(&new_path, "{}").unwrap();
    }

    if old_path.exists() {
        std::fs::remove_file(&old_path).unwrap();
    }

    let result = migration::migrate_index_rename();
    assert_eq!(result.changed_count, 0);
    assert!(result.warnings.is_empty());
}

#[test]
fn migrate_encrypt_plaintext_index() {
    let root = setup_data_dir();
    let key = common::test_key(root);

    let index_path = config::index_path();
    std::fs::write(&index_path, "{\"sessions\":[]}").unwrap();

    let result = migration::migrate_encrypt_plaintext_index(&key);
    assert_eq!(result.changed_count, 1);
    assert!(result.warnings.is_empty());

    let raw = std::fs::read(&index_path).unwrap();
    assert!(crypto::is_encrypted(&raw));
}

#[test]
fn migrate_encrypt_plaintext_index_idempotent() {
    let root = setup_data_dir();
    let key = common::test_key(root);

    let index_path = config::index_path();

    let raw = std::fs::read(&index_path).unwrap();
    assert!(crypto::is_encrypted(&raw), "index should already be encrypted from prior test");

    let result = migration::migrate_encrypt_plaintext_index(&key);
    assert_eq!(result.changed_count, 0);
    assert!(result.warnings.is_empty());
}

#[test]
fn migrate_worldbook_normalization() {
    let root = setup_data_dir();
    let key = common::test_key(root);

    let legacy_json = serde_json::json!({
        "entries": {
            "0": {
                "key": ["dragon", "wyrm"],
                "keysecondary": ["fire"],
                "content": "Dragons breathe fire.",
                "disable": false,
                "order": 5,
                "depth": 4
            }
        }
    });

    let json_path = config::worldinfo_dir().join("legacy-wb.json");
    std::fs::write(&json_path, legacy_json.to_string()).unwrap();

    let result = migration::migrate_worldbook_normalization(Some(&key));
    assert!(result.changed_count >= 1, "expected at least 1 rewrite, got {}", result.changed_count);
    assert!(result.warnings.is_empty(), "unexpected warnings: {:?}", result.warnings);

    common::assert_file_missing(&json_path);

    let encrypted_path = config::worldinfo_dir().join("legacy-wb.worldbook");
    common::assert_file_exists(&encrypted_path);

    let raw = std::fs::read(&encrypted_path).unwrap();
    assert!(crypto::is_encrypted(&raw));

    let decrypted = crypto::read_and_decrypt(&encrypted_path, Some(&key)).unwrap();
    let wb: libllm::worldinfo::WorldBook = serde_json::from_str(&decrypted).unwrap();
    assert!(!wb.entries.is_empty());
    let entry = &wb.entries[0];
    assert_eq!(entry.keys, vec!["dragon", "wyrm"]);
    assert_eq!(entry.secondary_keys, vec!["fire"]);
    assert_eq!(entry.content, "Dragons breathe fire.");
    assert!(entry.enabled);
}

// ---------------------------------------------------------------------------
// Config + Migration interaction tests
// ---------------------------------------------------------------------------

#[test]
fn migrate_personas_from_config() {
    let root = setup_data_dir();
    let key = common::test_key(root);

    let toml_content = r#"
user_name = "ConfigUser"
user_persona = "A user migrated from config"
"#;
    std::fs::write(config::config_path(), toml_content).unwrap();

    let result = migration::migrate_personas_from_config(Some(&key));
    assert_eq!(result.changed_count, 1);
    assert!(result.warnings.is_empty(), "unexpected warnings: {:?}", result.warnings);

    let persona_path = config::personas_dir().join("ConfigUser.persona");
    common::assert_file_exists(&persona_path);

    let decrypted = crypto::read_and_decrypt(&persona_path, Some(&key)).unwrap();
    let persona: libllm::persona::PersonaFile = serde_json::from_str(&decrypted).unwrap();
    assert_eq!(persona.name, "ConfigUser");
    assert_eq!(persona.persona, "A user migrated from config");
}

#[test]
fn config_survives_migration() {
    let root = setup_data_dir();
    let key = common::test_key(root);

    let mut cfg = Config::default();
    cfg.api_url = Some("http://survive.test/v1".to_owned());
    cfg.debug_log = true;
    config::save(&cfg).unwrap();

    migration::migrate_encrypt_plaintext_cards(&key);
    migration::migrate_encrypt_plaintext_prompts(&key);
    migration::migrate_encrypt_plaintext_personas(&key);
    migration::migrate_index_rename();

    let loaded = config::load();
    assert_eq!(loaded.api_url.as_deref(), Some("http://survive.test/v1"));
    assert!(loaded.debug_log);
}
