#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test helpers: a failed setup step should panic at its call site"
)]

use std::path::Path;

use libllm_core::character::CharacterCard;
use libllm_core::crypto::DerivedKey;
use libllm_core::persona::PersonaFile;
use libllm_core::sampling::SamplingOverrides;
use libllm_core::session::{Message, MessageTree, Role, Session};
use libllm_core::system_prompt::SystemPromptFile;
use libllm_core::worldinfo::{Entry, WorldBook};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Create a temporary directory for a single test.
///
/// Returns a `TempDir` guard that deletes the directory when dropped.
/// All test I/O should target paths under this directory.
pub fn temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("failed to create temp dir")
}

/// Derive an encryption key from a fixed test passkey and a fresh salt.
///
/// Writes `root/.salt`. Callers that also need to satisfy the encrypted-mode
/// marker invariant enforced by `validate_data_dir` must create `root/data.db`
/// (typically by opening a `Database` with the returned key).
pub fn test_key(root: &Path) -> DerivedKey {
    let salt_path = root.join(".salt");
    let salt = libllm_core::crypto::load_or_create_salt(&salt_path).expect("failed to create salt");
    libllm_core::crypto::derive_key("test-passkey", &salt).expect("failed to derive key")
}

/// Build a user message.
pub fn user_msg(content: &str) -> Message {
    Message::new(Role::User, content.to_string())
}

/// Build an assistant message.
pub fn assistant_msg(content: &str) -> Message {
    Message::new(Role::Assistant, content.to_string())
}

/// Build a system message.
pub fn system_msg(content: &str) -> Message {
    Message::new(Role::System, content.to_string())
}

/// Build a `Session` with a linear chain of messages (no branching).
///
/// Messages are pushed sequentially so each is a child of the previous.
pub fn linear_session(messages: Vec<Message>) -> Session {
    let mut tree = MessageTree::new();
    for m in messages {
        let parent = tree.head();
        tree.push(parent, m);
    }
    Session {
        tree,
        model: None,
        template: None,
        system_prompt: None,
        character: None,
        worldbooks: Vec::new(),
        persona: None,
        scenario: None,
        characters: Vec::new(),
        chat_mode: libllm_core::group_chat::ChatMode::default(),
        author_note: None,
    }
}

/// Build a minimal `CharacterCard` with only a name and description.
pub fn simple_character(name: &str, description: &str) -> CharacterCard {
    CharacterCard {
        name: name.to_string(),
        description: description.to_string(),
        personality: String::new(),
        scenario: String::new(),
        first_mes: String::new(),
        mes_example: String::new(),
        system_prompt: String::new(),
        post_history_instructions: String::new(),
        alternate_greetings: Vec::new(),
        author_note: None,
    }
}

/// Build a `WorldBook` with the given name and entries.
pub fn worldbook(name: &str, entries: Vec<Entry>) -> WorldBook {
    WorldBook {
        name: name.to_string(),
        entries,
    }
}

/// Build a single worldbook `Entry` with keyword triggers and content.
pub fn worldbook_entry(keys: Vec<&str>, content: &str) -> Entry {
    Entry {
        keys: keys.into_iter().map(String::from).collect(),
        secondary_keys: Vec::new(),
        selective: false,
        content: content.to_string(),
        constant: false,
        enabled: true,
        order: 0,
        depth: 4,
        case_sensitive: false,
    }
}

/// Build a `SystemPromptFile`.
pub fn system_prompt(name: &str, content: &str) -> SystemPromptFile {
    SystemPromptFile {
        name: name.to_string(),
        content: content.to_string(),
    }
}

/// Build a `PersonaFile`.
pub fn persona(name: &str, persona_text: &str) -> PersonaFile {
    PersonaFile {
        name: name.to_string(),
        persona: persona_text.to_string(),
    }
}

/// Build `SamplingOverrides` where every field is `None`.
pub fn empty_overrides() -> SamplingOverrides {
    SamplingOverrides {
        temperature: None,
        top_k: None,
        top_p: None,
        min_p: None,
        repeat_last_n: None,
        repeat_penalty: None,
        max_tokens: None,
    }
}

/// Write raw JSON to a file for testing import/parse flows.
pub fn write_json_file(path: &Path, json: &str) {
    std::fs::write(path, json).expect("failed to write JSON file");
}

/// Path to the compiled `client` binary, resolved at compile time by Cargo.
///
/// Used by integration tests that spawn the CLI as a subprocess.
pub fn client_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_libllm"))
}

/// Start a mock LLM server that returns a successful `/completions` response
/// containing `summary_text` in the `choices[0].text` field.
pub async fn start_mock_summarize_server(summary_text: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{"text": summary_text}]
        })))
        .mount(&server)
        .await;
    server
}

/// Seed a fresh plain-text database with the given (display_name, role, content) rows.
///
/// Opens `Database` first to run migrations, drops it, then inserts rows via a raw
/// `rusqlite::Connection` (which is callable from outside the `libllm` crate).
/// Returns the `TempDir` guard; callers must keep it alive for the duration of the test.
pub fn seed_search_db(rows: &[(&str, &str, &str)]) -> tempfile::TempDir {
    let dir = temp_dir();
    let db_path = dir.path().join("data.db");
    {
        let _db = libllm_storage::db::Database::open(&db_path, None).expect("open plain db");
    }
    let conn = rusqlite::Connection::open(&db_path).expect("open conn");
    let mut session_ids: std::collections::BTreeMap<String, i64> =
        std::collections::BTreeMap::new();
    for (display, role, content) in rows {
        let session_id = format!("sess-{display}");
        let display_owned = (*display).to_owned();
        if !session_ids.contains_key(&display_owned) {
            conn.execute(
                "INSERT INTO sessions (id, display_name, created_at, updated_at) \
                 VALUES (?1, ?2, 'now', 'now')",
                rusqlite::params![session_id, display],
            )
            .expect("insert session");
            session_ids.insert(display_owned.clone(), 0);
        }
        let next_id = session_ids.get_mut(&display_owned).unwrap();
        conn.execute(
            "INSERT INTO messages \
             (id, session_id, parent_id, preferred_child_id, role, content, timestamp) \
             VALUES (?1, ?2, NULL, NULL, ?3, ?4, '2026-01-01T00:00:00Z')",
            rusqlite::params![*next_id, session_id, role, content],
        )
        .expect("insert message");
        *next_id += 1;
    }
    dir
}

/// Opens (and migrates) a plaintext database at `db_path` and inserts one persona,
/// slug `alice`, named `alice`, so subcommand tests have a row to dump, restore, or list.
pub fn seed_persona_db(db_path: &Path) {
    let db = libllm_storage::db::Database::open(db_path, None).expect("open plain db");
    db.insert_persona(
        "alice",
        &PersonaFile {
            name: "alice".to_owned(),
            persona: "curious".to_owned(),
        },
    )
    .expect("insert alice");
}

/// Start a mock LLM server that returns HTTP 500 for every `/completions` request.
pub async fn start_mock_failing_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completions"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    server
}

/// Import a character card (minimal JSON) into `data_dir` using the CLI binary.
///
/// Creates a temporary JSON file named `{slug}.json`, spawns the `import` subcommand, and
/// asserts that it succeeds. The card name field is set to `name` so the slug stored in the
/// database matches `slugify(name)`. Callers must ensure the file stem is the slugified form
/// of `name` so that the slug stored in the DB matches what the test expects.
pub fn import_card(data_dir: &std::path::Path, slug: &str, name: &str) {
    let tmp = temp_dir();
    let card_path = tmp.path().join(format!("{slug}.json"));
    let json = format!(
        r#"{{"name":"{name}","description":"","personality":"","scenario":"","first_mes":"","mes_example":""}}"#
    );
    std::fs::write(&card_path, json).unwrap();
    let out = std::process::Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "import",
            card_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to spawn import");
    assert!(
        out.status.success(),
        "import {slug} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Import a persona (plain-text file) into `data_dir` using the CLI binary.
///
/// Creates a temporary `.txt` file named `{slug}.txt`, spawns `import --type persona`, and
/// asserts success. The persona text is minimal; only the name matters for CLI validation tests.
pub fn import_persona(data_dir: &std::path::Path, slug: &str, name: &str) {
    let tmp = temp_dir();
    let persona_path = tmp.path().join(format!("{slug}.txt"));
    std::fs::write(&persona_path, format!("Name: {name}\nA test persona.\n")).unwrap();
    let out = std::process::Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "import",
            "--type",
            "persona",
            persona_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to spawn import persona");
    assert!(
        out.status.success(),
        "import persona {slug} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
