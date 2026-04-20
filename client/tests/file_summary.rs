//! Integration tests for the file-summary cache feature.

#[path = "common/mod.rs"]
#[expect(dead_code, reason = "each test binary uses a different subset of common helpers")]
mod common;

use libllm::db::file_summaries::{self, FileSummaryStatus};
use libllm::files::{
    FileSummarizer, FileToSummarize, NullFileSummaryLookup, build_snapshot_body,
    content_hash_hex, snapshot_inner_text,
};
use libllm::session::{Message, Role};
use libllm::summarize::Summarizer;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

fn setup_summarizer_conn(session_id: &str) -> Arc<Mutex<Connection>> {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    libllm::db::schema::run_migrations(&conn).unwrap();
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
        libllm::client::ApiClient::new(&mock.uri(), true, libllm::config::Auth::None),
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
        libllm::client::ApiClient::new(&mock.uri(), true, libllm::config::Auth::None),
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
        libllm::client::ApiClient::new(&mock.uri(), true, libllm::config::Auth::None),
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
        libllm::client::ApiClient::new(&mock.uri(), true, libllm::config::Auth::None),
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
    let lookup = libllm::files::ScopedFileSummaryLookup {
        session_id: "s1",
        resolver: &summarizer,
    };
    let prompt = Summarizer::format_prompt("Summarise.", &refs, &lookup);
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
        libllm::client::ApiClient::new(&mock.uri(), true, libllm::config::Auth::None),
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
    let lookup = libllm::files::ScopedFileSummaryLookup {
        session_id: "s1",
        resolver: &summarizer,
    };
    let prompt = Summarizer::format_prompt("Summarise.", &refs, &lookup);
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
    let prompt = Summarizer::format_prompt("Summarise.", &refs, &NullFileSummaryLookup);
    assert!(prompt.contains("summary unavailable"));
    assert!(!prompt.contains("RAW_BODY_PRESENT"));
}
