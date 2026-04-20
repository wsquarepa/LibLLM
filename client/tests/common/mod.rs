use std::path::{Path, PathBuf};

use libllm::character::CharacterCard;
use libllm::crypto::DerivedKey;
use libllm::persona::PersonaFile;
use libllm::sampling::{SamplingOverrides, SamplingParams};
use libllm::session::{Message, MessageTree, Role, Session};
use libllm::system_prompt::SystemPromptFile;
use libllm::worldinfo::{Entry, WorldBook};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Create a temporary directory for a single test.
///
/// Returns a `TempDir` guard that deletes the directory when dropped.
/// All test I/O should target paths under this directory.
pub fn temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("failed to create temp dir")
}

/// Create the standard libllm subdirectory layout inside `root`.
///
/// Creates: sessions/, characters/, worldinfo/, system/, personas/
pub fn create_data_dirs(root: &Path) {
    let subdirs = ["sessions", "characters", "worldinfo", "system", "personas"];
    for sub in &subdirs {
        std::fs::create_dir_all(root.join(sub)).expect("failed to create subdir");
    }
}

/// Derive an encryption key from a fixed test passkey and a fresh salt.
///
/// Writes `root/.salt` and an empty `root/config.toml` so the directory satisfies the
/// strict encrypted-mode marker invariant enforced by `validate_data_dir`. Returns the
/// derived key.
pub fn test_key(root: &Path) -> DerivedKey {
    let salt_path = root.join(".salt");
    let salt = libllm::crypto::load_or_create_salt(&salt_path).expect("failed to create salt");
    let config_path = root.join("config.toml");
    if !config_path.exists() {
        std::fs::write(&config_path, "").expect("failed to create placeholder config.toml");
    }
    libllm::crypto::derive_key("test-passkey", &salt).expect("failed to derive key")
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
    }
}

/// Build a `CharacterCard` with all fields populated for thorough testing.
pub fn full_character() -> CharacterCard {
    CharacterCard {
        name: "TestChar".to_string(),
        description: "A test character for integration tests.".to_string(),
        personality: "Helpful and precise.".to_string(),
        scenario: "In a testing environment.".to_string(),
        first_mes: "Hello, I am TestChar.".to_string(),
        mes_example: "<START>\n{{user}}: Hi\n{{char}}: Hello!".to_string(),
        system_prompt: "You are TestChar.".to_string(),
        post_history_instructions: "Stay in character.".to_string(),
        alternate_greetings: vec!["Greetings!".to_string()],
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

/// Build a constant (always-active) worldbook entry.
pub fn constant_entry(content: &str) -> Entry {
    Entry {
        keys: Vec::new(),
        secondary_keys: Vec::new(),
        selective: false,
        content: content.to_string(),
        constant: true,
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

/// Build a `SamplingParams` with explicit values (no defaults).
pub fn sampling_params(
    temperature: f64,
    top_k: i64,
    top_p: f64,
    min_p: f64,
    repeat_last_n: i64,
    repeat_penalty: f64,
    max_tokens: i64,
) -> SamplingParams {
    SamplingParams {
        temperature,
        top_k,
        top_p,
        min_p,
        repeat_last_n,
        repeat_penalty,
        max_tokens,
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

/// Build a session file path inside the sessions subdirectory.
pub fn session_path(root: &Path, name: &str) -> PathBuf {
    root.join("sessions").join(name)
}

/// Write raw JSON to a file for testing import/parse flows.
pub fn write_json_file(path: &Path, json: &str) {
    std::fs::write(path, json).expect("failed to write JSON file");
}

/// Path to the compiled `client` binary, resolved at compile time by Cargo.
///
/// Used by integration tests that spawn the CLI as a subprocess.
pub fn client_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_client"))
}

/// Read a file to a string (panics on failure, test-only).
pub fn read_file(path: &Path) -> String {
    std::fs::read_to_string(path).expect("failed to read file")
}

/// Assert that a file exists at the given path.
pub fn assert_file_exists(path: &Path) {
    assert!(path.exists(), "expected file to exist: {}", path.display());
}

/// Assert that a file does NOT exist at the given path.
pub fn assert_file_missing(path: &Path) {
    assert!(
        !path.exists(),
        "expected file to not exist: {}",
        path.display()
    );
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
