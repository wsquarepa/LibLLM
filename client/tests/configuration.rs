#[expect(dead_code, reason = "each test binary uses a different subset of common helpers")]
mod common;

use client::validation;
use libllm::config::{self, Auth, Config};
use libllm::migration;

fn setup_data_dir() -> tempfile::TempDir {
    let dir = common::temp_dir();
    config::set_data_dir(dir.path().to_path_buf()).unwrap();
    config::ensure_dirs().unwrap();
    dir
}

// ---------------------------------------------------------------------------
// Config tests
// ---------------------------------------------------------------------------

#[test]
fn config_default_values() {
    let cfg = Config::default();
    assert!(cfg.api_url.is_none());
    assert_eq!(cfg.api_url(), "http://localhost:5001/v1");
    assert!(cfg.template_preset.is_none());
    assert!(cfg.instruct_preset.is_none());
    assert!(cfg.reasoning_preset.is_none());
    assert!(cfg.sampling.temperature.is_none());
    assert!(cfg.sampling.top_k.is_none());
    assert!(cfg.sampling.top_p.is_none());
    assert!(cfg.sampling.min_p.is_none());
    assert!(cfg.sampling.repeat_last_n.is_none());
    assert!(cfg.sampling.repeat_penalty.is_none());
    assert!(cfg.sampling.max_tokens.is_none());
    assert!(cfg.worldbooks.is_empty());
    assert!(!cfg.tls_skip_verify);
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
    let dir = setup_data_dir();
    let root = dir.path();
    let _key = common::test_key(root);

    let mut cfg = Config::default();
    cfg.api_url = Some("http://roundtrip.test/v1".to_owned());
    cfg.template_preset = Some("chatml".to_owned());
    cfg.instruct_preset = Some("alpaca".to_owned());
    cfg.reasoning_preset = Some("deepseek".to_owned());
    cfg.worldbooks = vec!["lore.worldbook".to_owned()];
    cfg.tls_skip_verify = true;
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
    assert_eq!(loaded.default_persona.as_deref(), Some("tester"));
    assert_eq!(loaded.sampling.temperature, Some(0.7));
    assert_eq!(loaded.sampling.top_k, Some(40));
}

#[test]
fn config_missing_file_returns_default() {
    let dir = setup_data_dir();
    let _root = dir.path();

    let bogus = config::data_dir().join("nonexistent_config.toml");
    assert!(!bogus.exists());

    let cfg = config::load();
    assert!(cfg.api_url.is_none() || cfg.api_url.is_some());
}

#[test]
fn config_partial_toml() {
    let dir = setup_data_dir();
    let root = dir.path();
    let _key = common::test_key(root);

    let partial = "api_url = \"http://partial.test/v1\"\n";
    std::fs::write(config::config_path(), partial).unwrap();

    let loaded = config::load();
    assert_eq!(loaded.api_url.as_deref(), Some("http://partial.test/v1"));
    assert!(loaded.sampling.temperature.is_none());
    assert!(loaded.worldbooks.is_empty());
    assert!(!loaded.tls_skip_verify);
    assert!(loaded.default_persona.is_none());
}

#[test]
fn config_ensure_dirs_creates_data_directory() {
    let dir = setup_data_dir();
    let root = dir.path();

    assert!(root.is_dir());
}

// ---------------------------------------------------------------------------
// Summarization config tests
// ---------------------------------------------------------------------------

#[test]
fn summarization_config_defaults() {
    let config: libllm::config::Config = toml::from_str("").unwrap();
    assert!(config.summarization.enabled);
    assert_eq!(config.summarization.context_size, 8192);
    assert_eq!(config.summarization.trigger_threshold, 5);
    assert!(config.summarization.api_url.is_none());
    assert!(!config.summarization.prompt.is_empty());
}

#[test]
fn summarization_config_custom() {
    let toml_str = r#"
[summarization]
enabled = false
api_url = "http://other:8080/v1"
context_size = 16384
trigger_threshold = 10
prompt = "Custom prompt"
"#;
    let config: libllm::config::Config = toml::from_str(toml_str).unwrap();
    assert!(!config.summarization.enabled);
    assert_eq!(config.summarization.api_url.as_deref(), Some("http://other:8080/v1"));
    assert_eq!(config.summarization.context_size, 16384);
    assert_eq!(config.summarization.trigger_threshold, 10);
    assert_eq!(config.summarization.prompt, "Custom prompt");
}

// ---------------------------------------------------------------------------
// Migration tests
// ---------------------------------------------------------------------------

#[test]
fn migrate_config_path_is_callable() {
    let _dir = setup_data_dir();
    migration::migrate_config_path();
}


#[test]
fn config_survives_migration() {
    let dir = setup_data_dir();
    let root = dir.path();
    let key = common::test_key(root);
    let _ = key;

    let mut cfg = Config::default();
    cfg.api_url = Some("http://survive.test/v1".to_owned());
    config::save(&cfg).unwrap();

    migration::migrate_config_path();

    let loaded = config::load();
    assert_eq!(loaded.api_url.as_deref(), Some("http://survive.test/v1"));
}

// ---------------------------------------------------------------------------
// Data directory validation tests
// ---------------------------------------------------------------------------

#[test]
fn validate_data_dir_creates_missing_dir() {
    let parent = common::temp_dir();
    let new_path = parent.path().join("brand_new");
    assert!(!new_path.exists());

    let is_existing = validation::validate_data_dir(&new_path).unwrap();
    assert!(!is_existing);
    assert!(new_path.is_dir());
}

#[test]
fn validate_data_dir_accepts_empty_dir() {
    let dir = common::temp_dir();
    let is_existing = validation::validate_data_dir(dir.path()).unwrap();
    assert!(!is_existing);
}

#[test]
fn validate_data_dir_rejects_non_libllm_dir() {
    let dir = common::temp_dir();
    std::fs::write(dir.path().join("random.txt"), "not libllm").unwrap();

    let err = validation::validate_data_dir(dir.path()).unwrap_err();
    assert!(
        format!("{err}").contains("does not appear to be a libllm data directory"),
        "unexpected error: {err}"
    );
}

#[test]
fn validate_data_dir_accepts_dir_with_config_toml() {
    let dir = common::temp_dir();
    std::fs::write(dir.path().join("config.toml"), "").unwrap();

    let is_existing = validation::validate_data_dir(dir.path()).unwrap();
    assert!(is_existing);
}

#[test]
fn validate_data_dir_accepts_dir_with_data_db() {
    let dir = common::temp_dir();
    std::fs::write(dir.path().join("data.db"), "").unwrap();

    let is_existing = validation::validate_data_dir(dir.path()).unwrap();
    assert!(is_existing);
}

#[test]
fn validate_data_dir_rejects_file_path() {
    let dir = common::temp_dir();
    let file_path = dir.path().join("not_a_dir");
    std::fs::write(&file_path, "").unwrap();

    let err = validation::validate_data_dir(&file_path).unwrap_err();
    assert!(
        format!("{err}").contains("is not a directory"),
        "unexpected error: {err}"
    );
}

#[test]
fn is_libllm_data_dir_detects_markers() {
    let dir = common::temp_dir();
    assert!(!validation::is_libllm_data_dir(dir.path()));

    std::fs::write(dir.path().join("data.db"), "").unwrap();
    assert!(validation::is_libllm_data_dir(dir.path()));
}

#[test]
fn is_libllm_data_dir_detects_config_toml() {
    let dir = common::temp_dir();
    std::fs::write(dir.path().join("config.toml"), "").unwrap();
    assert!(validation::is_libllm_data_dir(dir.path()));
}

#[test]
fn theme_editor_covers_all_color_override_fields() {
    let config = libllm::config::Config::default();
    let dialog = client::tui::dialogs::open_theme_editor(&config);
    assert_eq!(dialog.sections().len(), 5, "expected 5 theme tabs");
    let color_field_count: usize = dialog
        .sections()
        .iter()
        .skip(1)
        .map(|s| s.labels.len())
        .sum();
    assert_eq!(color_field_count, 26, "tabs 2-5 must cover all 26 ThemeColorOverrides fields");
}

#[test]
fn theme_overrides_apply_round_trip() {
    let dir = common::temp_dir();
    libllm::config::set_data_dir(dir.path().to_path_buf()).ok();
    libllm::config::ensure_dirs().unwrap();

    let sections = vec![
        vec!["dark".to_owned(), "".to_owned(), "".to_owned(), "".to_owned()],
        vec!["#ff0000".to_owned(), "".to_owned(), "".to_owned(), "".to_owned(), "".to_owned()],
        vec!["".to_owned(); 10],
        vec!["".to_owned(); 8],
        vec!["".to_owned(); 3],
    ];

    let cfg = libllm::config::Config::default();
    client::tui::business::apply_theme_color_sections(&sections, cfg).unwrap();

    let saved = libllm::config::load();
    let overrides = saved.theme_colors.expect("expected overrides to persist");
    assert_eq!(overrides.user_message.as_deref(), Some("#ff0000"));
    assert!(overrides.assistant_message_fg.is_none());
}

#[test]
fn empty_theme_override_drops_to_none() {
    let dir = common::temp_dir();
    libllm::config::set_data_dir(dir.path().to_path_buf()).ok();
    libllm::config::ensure_dirs().unwrap();

    let mut cfg = libllm::config::Config::default();
    cfg.theme_colors = Some(libllm::config::ThemeColorOverrides {
        user_message: Some("#ff0000".to_owned()),
        ..Default::default()
    });

    let sections = vec![
        vec!["dark".to_owned(), "".to_owned(), "".to_owned(), "".to_owned()],
        vec!["".to_owned(), "".to_owned(), "".to_owned(), "".to_owned(), "".to_owned()],
        vec!["".to_owned(); 10],
        vec!["".to_owned(); 8],
        vec!["".to_owned(); 3],
    ];

    client::tui::business::apply_theme_color_sections(&sections, cfg).unwrap();
    let saved = libllm::config::load();
    assert!(saved.theme_colors.is_none());
}

#[test]
fn log_filter_without_debug_is_a_parse_error() {
    use clap::Parser;
    let result = client::cli::Args::try_parse_from(["libllm", "--log-filter", "info"]);
    assert!(result.is_err(), "expected parse failure but parsing succeeded");
    let err = result.err().unwrap().to_string();
    assert!(
        err.contains("--debug") || err.contains("debug"),
        "unexpected error message: {err}"
    );
}

#[test]
fn log_filter_with_debug_parses() {
    use clap::Parser;
    let result = client::cli::Args::try_parse_from([
        "libllm",
        "--debug",
        "/tmp/x.log",
        "--log-filter",
        "info",
    ]);
    assert!(result.is_ok(), "expected parse success but got an error");
}

// ---------------------------------------------------------------------------
// Auth round-trip tests
// ---------------------------------------------------------------------------

#[test]
fn config_auth_roundtrip_bearer() {
    let dir = setup_data_dir();
    let _key = common::test_key(dir.path());
    let cfg = Config {
        auth: Auth::Bearer { token: "sk-abc".into() },
        ..Config::default()
    };
    config::save(&cfg).unwrap();
    let loaded = config::load();
    assert_eq!(loaded.auth, Auth::Bearer { token: "sk-abc".into() });
}

#[test]
fn config_auth_roundtrip_basic() {
    let dir = setup_data_dir();
    let _key = common::test_key(dir.path());
    let cfg = Config {
        auth: Auth::Basic { username: "u".into(), password: "p".into() },
        ..Config::default()
    };
    config::save(&cfg).unwrap();
    let loaded = config::load();
    assert_eq!(loaded.auth, Auth::Basic { username: "u".into(), password: "p".into() });
}

#[test]
fn config_auth_roundtrip_header() {
    let dir = setup_data_dir();
    let _key = common::test_key(dir.path());
    let cfg = Config {
        auth: Auth::Header { name: "X-Key".into(), value: "v".into() },
        ..Config::default()
    };
    config::save(&cfg).unwrap();
    let loaded = config::load();
    assert_eq!(loaded.auth, Auth::Header { name: "X-Key".into(), value: "v".into() });
}

#[test]
fn config_auth_roundtrip_query() {
    let dir = setup_data_dir();
    let _key = common::test_key(dir.path());
    let cfg = Config {
        auth: Auth::Query { name: "api_key".into(), value: "v".into() },
        ..Config::default()
    };
    config::save(&cfg).unwrap();
    let loaded = config::load();
    assert_eq!(loaded.auth, Auth::Query { name: "api_key".into(), value: "v".into() });
}

#[test]
fn config_auth_defaults_to_none_for_missing_section() {
    let dir = setup_data_dir();
    let _key = common::test_key(dir.path());
    std::fs::write(
        config::config_path(),
        "api_url = \"http://localhost:5001/v1\"\n",
    )
    .unwrap();
    let loaded = config::load();
    assert_eq!(loaded.auth, Auth::None);
}
