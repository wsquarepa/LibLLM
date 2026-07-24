//! Integration tests for the file-summary cache feature.

#[path = "common/mod.rs"]
#[expect(
    dead_code,
    reason = "each test binary uses a different subset of common helpers"
)]
mod common;

use libllm_core::files::{
    FileToSummarize, NullFileSummaryLookup, build_snapshot_body, content_hash_hex,
    snapshot_inner_text,
};
use libllm_core::session::{Message, Role};
use libllm_protocol::summarize::Summarizer;
use libllm_storage::db::file_summaries::{self, FileSummaryStatus};
use libllm_tui::file_summarizer::FileSummarizer;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

fn setup_summarizer_conn(session_id: &str) -> Arc<Mutex<Connection>> {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    libllm_storage::db::migrations::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO sessions (id, created_at, updated_at) VALUES (?1, 'now', 'now')",
        rusqlite::params![session_id],
    )
    .unwrap();
    Arc::new(Mutex::new(conn))
}

#[tokio::test]
async fn eager_schedule_transitions_to_done_with_mocked_summary() {
    let mock = common::start_mock_summarize_server("This is the summary.").await;
    let conn = setup_summarizer_conn("s1");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let summarizer = FileSummarizer::new(
        Arc::clone(&conn),
        libllm_protocol::client::ApiClient::new(&mock.uri(), true, libllm_core::config::Auth::None),
        "Summarize the file.".to_owned(),
        tx,
    );

    let file = FileToSummarize {
        basename: "a.md".to_owned(),
        content_hash: "hash-a".to_owned(),
        body: "raw file body".to_owned(),
    };
    summarizer.schedule("s1", &file);

    let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("summarizer should emit a ReadyEvent")
        .expect("channel not closed");
    assert_eq!(event.status, FileSummaryStatus::Done);

    let row = file_summaries::lookup(&conn.lock().unwrap(), "s1", "hash-a")
        .unwrap()
        .unwrap();
    assert_eq!(row.status, FileSummaryStatus::Done);
    assert_eq!(row.summary, "This is the summary.");
}

#[tokio::test]
async fn permanent_failure_transitions_to_failed() {
    let mock = common::start_mock_failing_server().await;
    let conn = setup_summarizer_conn("s1");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let summarizer = FileSummarizer::new(
        Arc::clone(&conn),
        libllm_protocol::client::ApiClient::new(&mock.uri(), true, libllm_core::config::Auth::None),
        "Summarize the file.".to_owned(),
        tx,
    );

    let file = FileToSummarize {
        basename: "a.md".to_owned(),
        content_hash: "hash-a".to_owned(),
        body: "raw file body".to_owned(),
    };
    summarizer.schedule("s1", &file);

    let event = tokio::time::timeout(std::time::Duration::from_secs(30), rx.recv())
        .await
        .expect("summarizer should emit a ReadyEvent")
        .expect("channel not closed");
    assert_eq!(event.status, FileSummaryStatus::Failed);

    let row = file_summaries::lookup(&conn.lock().unwrap(), "s1", "hash-a")
        .unwrap()
        .unwrap();
    assert_eq!(row.status, FileSummaryStatus::Failed);
}

#[tokio::test]
async fn ensure_ready_waits_for_pending_then_resolves() {
    let mock = common::start_mock_summarize_server("delayed summary").await;
    let conn = setup_summarizer_conn("s1");
    let (tx, _rx) = mpsc::unbounded_channel();
    let summarizer = FileSummarizer::new(
        Arc::clone(&conn),
        libllm_protocol::client::ApiClient::new(&mock.uri(), true, libllm_core::config::Auth::None),
        "Summarize the file.".to_owned(),
        tx,
    );

    let file = FileToSummarize {
        basename: "a.md".to_owned(),
        content_hash: "hash-a".to_owned(),
        body: "raw file body".to_owned(),
    };
    summarizer.schedule("s1", &file);
    summarizer
        .ensure_ready("s1", std::slice::from_ref(&file))
        .await
        .unwrap();

    let row = file_summaries::lookup(&conn.lock().unwrap(), "s1", "hash-a")
        .unwrap()
        .unwrap();
    assert_eq!(row.status, FileSummaryStatus::Done);
    assert_eq!(row.summary, "delayed summary");
}

#[tokio::test]
async fn summary_substitution_in_summarize_prompt_hides_raw_body() {
    let mock = common::start_mock_summarize_server("FILE_SUMMARY").await;
    let conn = setup_summarizer_conn("s1");
    let (tx, _rx) = mpsc::unbounded_channel();
    let summarizer = FileSummarizer::new(
        Arc::clone(&conn),
        libllm_protocol::client::ApiClient::new(&mock.uri(), true, libllm_core::config::Auth::None),
        "Summarize the file.".to_owned(),
        tx,
    );

    let snapshot_body = build_snapshot_body("doc.md", "SECRET_RAW_CONTENT");
    let inner = snapshot_inner_text(&snapshot_body).to_owned();
    let hash = content_hash_hex(inner.as_bytes());
    let file = FileToSummarize {
        basename: "doc.md".to_owned(),
        content_hash: hash,
        body: inner,
    };
    summarizer.schedule("s1", &file);
    summarizer
        .ensure_ready("s1", std::slice::from_ref(&file))
        .await
        .unwrap();

    let msgs = [
        Message::new(Role::User, "hi".to_owned()),
        Message::new(Role::System, snapshot_body),
        Message::new(Role::Assistant, "reply".to_owned()),
    ];
    let refs: Vec<&Message> = msgs.iter().collect();
    let lookup = libllm_core::files::ScopedFileSummaryLookup {
        session_id: "s1",
        resolver: &summarizer,
    };
    let prompt = Summarizer::format_prompt(None, "Summarise.", &refs, &lookup);
    assert!(prompt.contains("FILE_SUMMARY"));
    assert!(!prompt.contains("SECRET_RAW_CONTENT"));
}

#[tokio::test]
async fn failed_summary_produces_placeholder_in_prompt() {
    let mock = common::start_mock_failing_server().await;
    let conn = setup_summarizer_conn("s1");
    let (tx, _rx) = mpsc::unbounded_channel();
    let summarizer = FileSummarizer::new(
        Arc::clone(&conn),
        libllm_protocol::client::ApiClient::new(&mock.uri(), true, libllm_core::config::Auth::None),
        "Summarize the file.".to_owned(),
        tx,
    );

    let snapshot_body = build_snapshot_body("doc.md", "SECRET_RAW_CONTENT");
    let inner = snapshot_inner_text(&snapshot_body).to_owned();
    let hash = content_hash_hex(inner.as_bytes());
    let file = FileToSummarize {
        basename: "doc.md".to_owned(),
        content_hash: hash,
        body: inner,
    };
    summarizer.schedule("s1", &file);
    summarizer
        .ensure_ready("s1", std::slice::from_ref(&file))
        .await
        .unwrap();

    let msgs = [Message::new(Role::System, snapshot_body)];
    let refs: Vec<&Message> = msgs.iter().collect();
    let lookup = libllm_core::files::ScopedFileSummaryLookup {
        session_id: "s1",
        resolver: &summarizer,
    };
    let prompt = Summarizer::format_prompt(None, "Summarise.", &refs, &lookup);
    assert!(prompt.contains("summary unavailable"));
    assert!(!prompt.contains("SECRET_RAW_CONTENT"));
}

#[tokio::test]
async fn cascade_delete_removes_summary_rows() {
    let conn = setup_summarizer_conn("s1");
    {
        let guard = conn.lock().unwrap();
        file_summaries::insert_pending(&guard, "s1", "hash-a", "a.md").unwrap();
    }
    assert!(
        file_summaries::lookup(&conn.lock().unwrap(), "s1", "hash-a")
            .unwrap()
            .is_some()
    );

    conn.lock()
        .unwrap()
        .execute("DELETE FROM sessions WHERE id = 's1'", [])
        .unwrap();
    assert!(
        file_summaries::lookup(&conn.lock().unwrap(), "s1", "hash-a")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn null_lookup_renders_placeholder() {
    let snapshot_body = build_snapshot_body("doc.md", "RAW_BODY_PRESENT");
    let msgs = [Message::new(Role::System, snapshot_body)];
    let refs: Vec<&Message> = msgs.iter().collect();
    let prompt = Summarizer::format_prompt(None, "Summarise.", &refs, &NullFileSummaryLookup);
    assert!(prompt.contains("summary unavailable"));
    assert!(!prompt.contains("RAW_BODY_PRESENT"));
}

#[tokio::test]
async fn lookup_on_empty_session_returns_none() {
    let conn = setup_summarizer_conn("s1");
    let (tx, _rx) = mpsc::unbounded_channel();
    let summarizer = FileSummarizer::new(
        Arc::clone(&conn),
        libllm_protocol::client::ApiClient::new(
            "http://127.0.0.1:1",
            true,
            libllm_core::config::Auth::None,
        ),
        "Summarize the file.".to_owned(),
        tx,
    );
    // Simulates the single-run path: the controller never calls schedule when
    // save_mode.id() returns None. Confirm lookup is safe for non-existent sessions.
    assert!(
        summarizer
            .lookup("nonexistent-session", "any-hash")
            .is_none()
    );
}

#[tokio::test]
async fn no_rows_when_schedule_is_never_called() {
    let conn = setup_summarizer_conn("s1");
    let (tx, _rx) = mpsc::unbounded_channel();
    let _summarizer = FileSummarizer::new(
        Arc::clone(&conn),
        libllm_protocol::client::ApiClient::new(
            "http://127.0.0.1:1",
            true,
            libllm_core::config::Auth::None,
        ),
        "Summarize the file.".to_owned(),
        tx,
    );

    let count: i64 = conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM file_summaries", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn fresh_encrypted_session_schedules_after_unlock_save() {
    let mock = common::start_mock_summarize_server("fresh session summary").await;

    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("data.db");
    let salt = [0u8; 16];
    let key = Arc::new(libllm_core::crypto::derive_key("test-passkey", &salt).unwrap());
    let session_id = "fresh-session";

    {
        let mut db = libllm_storage::db::Database::open(&db_path, Some(&key)).unwrap();
        let empty_session = libllm_core::session::Session {
            tree: libllm_core::session::MessageTree::new(),
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
        };
        db.save_session(session_id, &empty_session).unwrap();
    }

    let mut config = libllm_core::config::Config::default();
    config.summarization.api_url = Some(mock.uri());
    let cli_overrides = libllm_cli::cli::CliOverrides::default();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let summarizer = libllm_tui::business::build_file_summarizer(
        &db_path,
        Some(&key),
        &config,
        &cli_overrides,
        tx,
    )
    .unwrap();

    let file = FileToSummarize {
        basename: "a.md".to_owned(),
        content_hash: "hash-a".to_owned(),
        body: "raw file body".to_owned(),
    };
    summarizer.schedule(session_id, &file);

    let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("summarizer should emit a ReadyEvent")
        .expect("channel not closed");
    assert_eq!(event.status, FileSummaryStatus::Done);
    assert_eq!(event.session_id, session_id);
}

#[test]
fn build_file_summarizer_opens_encrypted_db() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("data.db");
    let salt = [0u8; 16];
    let key = Arc::new(libllm_core::crypto::derive_key("test-passkey", &salt).unwrap());

    let _ = libllm_storage::db::Database::open(&db_path, Some(&key)).unwrap();

    let (tx, _rx) = mpsc::unbounded_channel();
    let config = libllm_core::config::Config::default();
    let cli_overrides = libllm_cli::cli::CliOverrides::default();

    let summarizer = libllm_tui::business::build_file_summarizer(
        &db_path,
        Some(&key),
        &config,
        &cli_overrides,
        tx,
    )
    .expect("helper must open the encrypted DB");

    assert!(
        summarizer
            .lookup("nonexistent-session", "nonexistent-hash")
            .is_none()
    );
}

#[tokio::test]
async fn shutdown_then_schedule_drops_work() {
    let conn = setup_summarizer_conn("s2");
    let (tx, _rx) = mpsc::unbounded_channel();
    let summarizer = FileSummarizer::new(
        Arc::clone(&conn),
        libllm_protocol::client::ApiClient::new(
            "http://127.0.0.1:1",
            true,
            libllm_core::config::Auth::None,
        ),
        "Summarize the file.".to_owned(),
        tx,
    );

    summarizer.shutdown().await;

    let file = FileToSummarize {
        basename: "c.md".to_owned(),
        content_hash: "hash-c-shutdown".to_owned(),
        body: "body c".to_owned(),
    };
    summarizer.schedule("s2", &file);

    assert!(
        file_summaries::lookup(&conn.lock().unwrap(), "s2", "hash-c-shutdown")
            .unwrap()
            .is_none(),
        "schedule must drop work once shutdown latch is set"
    );
}

/// Destroy All always quiesces the summarizer before snapshot. When the
/// snapshot then fails, the UI re-inits a fresh instance on the same connection
/// so scheduling works again without a process restart.
#[tokio::test]
async fn reinit_after_shutdown_restores_schedule() {
    let conn = setup_summarizer_conn("s-reinit");
    let (tx, _rx) = mpsc::unbounded_channel();
    let summarizer = FileSummarizer::new(
        Arc::clone(&conn),
        libllm_protocol::client::ApiClient::new(
            "http://127.0.0.1:1",
            true,
            libllm_core::config::Auth::None,
        ),
        "Summarize the file.".to_owned(),
        tx.clone(),
    );

    let file_pre = FileToSummarize {
        basename: "pre.md".to_owned(),
        content_hash: "hash-pre".to_owned(),
        body: "body pre".to_owned(),
    };
    summarizer.schedule("s-reinit", &file_pre);
    assert!(
        file_summaries::lookup(&conn.lock().unwrap(), "s-reinit", "hash-pre")
            .unwrap()
            .is_some(),
        "pre-condition: schedule must work before shutdown"
    );

    // Destroy All quiesce step.
    summarizer.shutdown().await;
    let file_dead = FileToSummarize {
        basename: "dead.md".to_owned(),
        content_hash: "hash-dead".to_owned(),
        body: "body dead".to_owned(),
    };
    summarizer.schedule("s-reinit", &file_dead);
    assert!(
        file_summaries::lookup(&conn.lock().unwrap(), "s-reinit", "hash-dead")
            .unwrap()
            .is_none(),
        "shut-down instance must refuse new work"
    );

    // Snapshot-failed recovery path: install a new instance on the same conn
    // (mirrors business::reinit_file_summarizer_after_failed_snapshot).
    let restored = FileSummarizer::new(
        summarizer.conn_clone_for_reload(),
        libllm_protocol::client::ApiClient::new(
            "http://127.0.0.1:1",
            true,
            libllm_core::config::Auth::None,
        ),
        "Summarize the file.".to_owned(),
        summarizer.ready_tx_clone_for_reload(),
    );
    let file_post = FileToSummarize {
        basename: "post.md".to_owned(),
        content_hash: "hash-post".to_owned(),
        body: "body post".to_owned(),
    };
    restored.schedule("s-reinit", &file_post);
    assert!(
        file_summaries::lookup(&conn.lock().unwrap(), "s-reinit", "hash-post")
            .unwrap()
            .is_some(),
        "re-inited summarizer must accept schedule after failed destroy-all snapshot"
    );
}
