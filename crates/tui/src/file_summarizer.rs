//! Background LLM summarisation orchestrator.
//!
//! Drives scheduling, persistence, and ready-event broadcasting for the
//! file-summary cache feature. Pure types and lookup traits live in
//! `libllm_core::files`; this module owns only the stateful orchestrator that
//! depends on both the database and the HTTP client.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use rusqlite::Connection;
use tokio::sync::mpsc;

use libllm_core::files::{
    FileError, FileSummary, FileSummaryStatus, FileToSummarize, ReadyEvent, SessionScopedLookup,
};
use libllm_core::sampling::SamplingParams;
use libllm_protocol::client::ApiClient;
use libllm_storage::db::file_summaries;

const SUMMARY_STOP_TOKENS: &[&str] = &["\nUser:", "\nAssistant:", "\nSystem:"];
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const DEFAULT_PER_FILE_TIMEOUT: Duration = Duration::from_secs(60);
const SUMMARY_API_RETRIES: u32 = 2;

/// Orchestrates background LLM summarisation of attached file snapshots,
/// writing results to the `file_summaries` SQLite table via a dedicated
/// `rusqlite::Connection` (isolated from `App.db`) and broadcasting
/// `ReadyEvent`s on state transitions.
pub struct FileSummarizer {
    pub(crate) conn: Arc<Mutex<Connection>>,
    client: ApiClient,
    prompt: String,
    pub(crate) poll_interval: Duration,
    pub(crate) per_file_timeout: Duration,
    pub(crate) ready_tx: mpsc::UnboundedSender<ReadyEvent>,
    pub(crate) shutting_down: Arc<AtomicBool>,
    pub(crate) in_flight: Arc<AtomicUsize>,
}

impl FileSummarizer {
    pub fn new(
        conn: Arc<Mutex<Connection>>,
        client: ApiClient,
        prompt: String,
        ready_tx: mpsc::UnboundedSender<ReadyEvent>,
    ) -> Self {
        Self {
            conn,
            client,
            prompt,
            poll_interval: DEFAULT_POLL_INTERVAL,
            per_file_timeout: DEFAULT_PER_FILE_TIMEOUT,
            ready_tx,
            shutting_down: Arc::new(AtomicBool::new(false)),
            in_flight: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Schedules summarisation for one file. Idempotent: if a row already
    /// exists for `(session_id, content_hash)`, no task is spawned. Silently
    /// skips scheduling when `shutdown()` has been called.
    pub fn schedule(&self, session_id: &str, file: &FileToSummarize) {
        if self.shutting_down.load(Ordering::SeqCst) {
            tracing::debug!(
                session_id = %session_id,
                content_hash = %file.content_hash,
                "files.summary.schedule.skipped_shutdown"
            );
            return;
        }

        let inserted = {
            let guard = match self.conn.lock() {
                Ok(g) => g,
                Err(err) => {
                    tracing::error!(
                        result = "error",
                        session_id = %session_id,
                        content_hash = %file.content_hash,
                        error = %err,
                        "files.summary.schedule.lock_poisoned"
                    );
                    return;
                }
            };
            match file_summaries::insert_pending(
                &guard,
                session_id,
                &file.content_hash,
                &file.basename,
            ) {
                Ok(v) => v,
                Err(err) => {
                    tracing::error!(
                        result = "error",
                        session_id = %session_id,
                        content_hash = %file.content_hash,
                        error = %err,
                        "files.summary.schedule"
                    );
                    return;
                }
            }
        };
        if !inserted {
            tracing::debug!(
                session_id = %session_id,
                content_hash = %file.content_hash,
                "files.summary.schedule.skipped_existing"
            );
            return;
        }
        tracing::info!(
            result = "scheduled",
            session_id = %session_id,
            content_hash = %file.content_hash,
            basename = %file.basename,
            body_bytes = file.body.len(),
            "files.summary.schedule"
        );

        let conn = Arc::clone(&self.conn);
        let client = self.client.clone();
        let prompt_instruction = self.prompt.clone();
        let ready_tx = self.ready_tx.clone();
        let session_id = session_id.to_owned();
        let content_hash = file.content_hash.clone();
        let body = file.body.clone();
        let per_file_timeout = self.per_file_timeout;
        let in_flight = Arc::clone(&self.in_flight);
        in_flight.fetch_add(1, Ordering::SeqCst);
        tokio::spawn(async move {
            tracing::debug!(
                session_id = %session_id,
                content_hash = %content_hash,
                body_bytes = body.len(),
                "files.summary.task.start"
            );
            let outcome =
                run_summary_task(&client, &prompt_instruction, &body, per_file_timeout).await;
            let status = match &outcome {
                Ok(text) => {
                    let guard = conn.lock().expect("summarizer conn poisoned");
                    if let Err(err) =
                        file_summaries::set_done(&guard, &session_id, &content_hash, text)
                    {
                        tracing::error!(
                            result = "error",
                            error = %err,
                            session_id = %session_id,
                            content_hash = %content_hash,
                            "files.summary.persist_done"
                        );
                    }
                    FileSummaryStatus::Done
                }
                Err(err) => {
                    tracing::warn!(
                        result = "failed",
                        error = %err,
                        session_id = %session_id,
                        content_hash = %content_hash,
                        "files.summary.api"
                    );
                    let guard = conn.lock().expect("summarizer conn poisoned");
                    if let Err(set_err) =
                        file_summaries::set_failed(&guard, &session_id, &content_hash)
                    {
                        tracing::error!(
                            result = "error",
                            error = %set_err,
                            session_id = %session_id,
                            content_hash = %content_hash,
                            "files.summary.persist_failed"
                        );
                    }
                    FileSummaryStatus::Failed
                }
            };
            tracing::info!(
                session_id = %session_id,
                content_hash = %content_hash,
                status = ?status,
                "files.summary.task.done"
            );
            let _ = ready_tx.send(ReadyEvent {
                session_id,
                content_hash,
                status,
            });
            in_flight.fetch_sub(1, Ordering::SeqCst);
        });
    }

    /// Signals shutdown and waits up to 2 seconds for in-flight tasks to complete.
    ///
    /// After this returns, `schedule` will silently drop any new requests on
    /// **this** instance. Destroy All awaits this before `snapshot_data_dir` so
    /// the recovery archive is not taken while summarizer tasks can still write
    /// SQLite/WAL. On snapshot failure the UI replaces the instance (fresh latch)
    /// rather than clearing the latch in place.
    pub async fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while self.in_flight.load(Ordering::SeqCst) > 0 {
            if std::time::Instant::now() >= deadline {
                tracing::warn!(
                    in_flight = self.in_flight.load(Ordering::SeqCst),
                    "files.summary.shutdown.timeout"
                );
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// Resolves once every file in `files` is `done` or `failed`.
    /// Lazy-schedules any missing rows before waiting. Force-transitions
    /// stuck `pending` rows to `failed` after
    /// `per_file_timeout * files.len()` has elapsed.
    pub async fn ensure_ready(&self, session_id: &str, files: &[FileToSummarize]) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }

        for file in files {
            if self.lookup(session_id, &file.content_hash).is_none() {
                self.schedule(session_id, file);
            }
        }

        let start = std::time::Instant::now();
        let deadline = self.per_file_timeout * (files.len() as u32).max(1);
        loop {
            let pending: Vec<&FileToSummarize> = files
                .iter()
                .filter(|f| {
                    matches!(
                        self.lookup(session_id, &f.content_hash).map(|r| r.status),
                        Some(FileSummaryStatus::Pending)
                    )
                })
                .collect();
            if pending.is_empty() {
                tracing::info!(
                    result = "ready",
                    session_id = %session_id,
                    file_count = files.len(),
                    elapsed_ms = start.elapsed().as_secs_f64() * 1000.0,
                    "files.summary.ensure_ready"
                );
                return Ok(());
            }
            if start.elapsed() > deadline {
                let guard = self.conn.lock().expect("summarizer conn poisoned");
                for f in pending {
                    tracing::warn!(
                        result = "timeout",
                        session_id = %session_id,
                        content_hash = %f.content_hash,
                        "files.summary.ensure_ready"
                    );
                    file_summaries::set_failed(&guard, session_id, &f.content_hash)?;
                    let _ = self.ready_tx.send(ReadyEvent {
                        session_id: session_id.to_owned(),
                        content_hash: f.content_hash.clone(),
                        status: FileSummaryStatus::Failed,
                    });
                }
                return Ok(());
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }

    /// Thin wrapper that checks a resolved file against the summarizer's configured prompt.
    pub async fn check_fits(
        &self,
        counter: &libllm_protocol::tokenizer::TokenCounter,
        file: &libllm_core::files::ResolvedFile,
        context_size: usize,
    ) -> Result<(), FileError> {
        libllm_protocol::summarize::check_file_fits(counter, file, &self.prompt, context_size).await
    }

    #[cfg(test)]
    pub fn set_per_file_timeout_for_test(&mut self, d: std::time::Duration) {
        self.per_file_timeout = d;
        self.poll_interval = std::time::Duration::from_millis(10);
    }

    #[doc(hidden)]
    pub fn conn_clone_for_reload(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }

    #[doc(hidden)]
    pub fn ready_tx_clone_for_reload(&self) -> mpsc::UnboundedSender<ReadyEvent> {
        self.ready_tx.clone()
    }

    /// Synchronous DB lookup through the dedicated connection.
    pub fn lookup(&self, session_id: &str, content_hash: &str) -> Option<FileSummary> {
        let guard = match self.conn.lock() {
            Ok(g) => g,
            Err(err) => {
                tracing::error!(
                    result = "error",
                    session_id = %session_id,
                    content_hash = %content_hash,
                    error = %err,
                    "files.summary.lookup.lock_poisoned"
                );
                return None;
            }
        };
        match file_summaries::lookup(&guard, session_id, content_hash) {
            Ok(Some(row)) => {
                tracing::debug!(
                    session_id = %session_id,
                    content_hash = %content_hash,
                    status = ?row.status,
                    summary_bytes = row.summary.len(),
                    "files.summary.lookup.hit"
                );
                Some(FileSummary {
                    basename: row.basename,
                    summary: row.summary,
                    status: row.status,
                })
            }
            Ok(None) => {
                tracing::debug!(
                    session_id = %session_id,
                    content_hash = %content_hash,
                    "files.summary.lookup.miss"
                );
                None
            }
            Err(err) => {
                tracing::error!(
                    result = "error",
                    session_id = %session_id,
                    content_hash = %content_hash,
                    error = %err,
                    "files.summary.lookup"
                );
                None
            }
        }
    }
}

impl SessionScopedLookup for FileSummarizer {
    fn lookup(&self, session_id: &str, content_hash: &str) -> Option<FileSummary> {
        Self::lookup(self, session_id, content_hash)
    }
}

async fn run_summary_task(
    client: &ApiClient,
    instruction: &str,
    body: &str,
    per_call_timeout: Duration,
) -> Result<String> {
    let prompt = format!("--- FILE ---\n{body}\n--- END FILE ---\n\n{instruction}\n\nSummary:");
    let sampling = SamplingParams {
        temperature: 0.3,
        max_tokens: 512,
        ..SamplingParams::default()
    };

    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..=SUMMARY_API_RETRIES {
        let call = client.complete(&prompt, SUMMARY_STOP_TOKENS, &sampling);
        match tokio::time::timeout(per_call_timeout, call).await {
            Ok(Ok(text)) => return Ok(text.trim().to_owned()),
            Ok(Err(err)) => {
                tracing::warn!(
                    result = "retry",
                    attempt = attempt,
                    error = %err,
                    "files.summary.api"
                );
                last_err = Some(err.into());
            }
            Err(_) => {
                tracing::warn!(
                    result = "timeout",
                    attempt = attempt,
                    timeout_ms = per_call_timeout.as_millis() as u64,
                    "files.summary.api"
                );
                last_err = Some(anyhow::anyhow!(
                    "summary API call timed out after {:?}",
                    per_call_timeout
                ));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("summary API call failed")))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use libllm_core::files::{FileSummaryStatus, FileToSummarize};
    use rusqlite::Connection;
    use tokio::sync::mpsc;

    use super::FileSummarizer;

    fn summarizer_conn() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        libllm_storage::db::migrations::run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s1', 'now', 'now')",
            [],
        )
        .unwrap();
        Arc::new(Mutex::new(conn))
    }

    #[tokio::test]
    async fn summarizer_schedule_inserts_pending_row() {
        let conn = summarizer_conn();
        let (tx, _rx) = mpsc::unbounded_channel();
        let summarizer = FileSummarizer::new(
            Arc::clone(&conn),
            libllm_protocol::client::ApiClient::new(
                "http://127.0.0.1:1",
                true,
                libllm_core::config::Auth::None,
            ),
            "summarize this".to_owned(),
            tx,
        );
        let file = FileToSummarize {
            basename: "a.md".to_owned(),
            content_hash: "h1".to_owned(),
            body: "hello world".to_owned(),
        };
        summarizer.schedule("s1", &file);

        let row = libllm_storage::db::file_summaries::lookup(&conn.lock().unwrap(), "s1", "h1")
            .unwrap()
            .unwrap();
        assert_eq!(row.status, FileSummaryStatus::Pending);
    }

    #[tokio::test]
    async fn summarizer_schedule_is_idempotent() {
        let conn = summarizer_conn();
        let (tx, _rx) = mpsc::unbounded_channel();
        let summarizer = FileSummarizer::new(
            Arc::clone(&conn),
            libllm_protocol::client::ApiClient::new(
                "http://127.0.0.1:1",
                true,
                libllm_core::config::Auth::None,
            ),
            "summarize this".to_owned(),
            tx,
        );
        let file = FileToSummarize {
            basename: "a.md".to_owned(),
            content_hash: "h1".to_owned(),
            body: "hello world".to_owned(),
        };
        summarizer.schedule("s1", &file);
        summarizer.schedule("s1", &file);

        let count: i64 = conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM file_summaries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn summarizer_lookup_returns_pending_row() {
        let conn = summarizer_conn();
        let (tx, _rx) = mpsc::unbounded_channel();
        let summarizer = FileSummarizer::new(
            Arc::clone(&conn),
            libllm_protocol::client::ApiClient::new(
                "http://127.0.0.1:1",
                true,
                libllm_core::config::Auth::None,
            ),
            "summarize this".to_owned(),
            tx,
        );
        let file = FileToSummarize {
            basename: "a.md".to_owned(),
            content_hash: "h1".to_owned(),
            body: "hello world".to_owned(),
        };
        summarizer.schedule("s1", &file);

        let got = summarizer.lookup("s1", "h1").unwrap();
        assert_eq!(got.status, FileSummaryStatus::Pending);
        assert_eq!(got.basename, "a.md");
    }

    #[tokio::test]
    async fn ensure_ready_returns_immediately_when_no_files() {
        let conn = summarizer_conn();
        let (tx, _rx) = mpsc::unbounded_channel();
        let summarizer = FileSummarizer::new(
            Arc::clone(&conn),
            libllm_protocol::client::ApiClient::new(
                "http://127.0.0.1:1",
                true,
                libllm_core::config::Auth::None,
            ),
            "summarize this".to_owned(),
            tx,
        );
        summarizer.ensure_ready("s1", &[]).await.unwrap();
    }

    #[tokio::test]
    async fn ensure_ready_schedules_missing_rows() {
        let conn = summarizer_conn();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut summarizer = FileSummarizer::new(
            Arc::clone(&conn),
            libllm_protocol::client::ApiClient::new(
                "http://127.0.0.1:1",
                true,
                libllm_core::config::Auth::None,
            ),
            "summarize this".to_owned(),
            tx,
        );
        summarizer.set_per_file_timeout_for_test(std::time::Duration::from_millis(200));

        let file = FileToSummarize {
            basename: "a.md".to_owned(),
            content_hash: "h1".to_owned(),
            body: "body".to_owned(),
        };

        summarizer
            .ensure_ready("s1", std::slice::from_ref(&file))
            .await
            .unwrap();
        let row = libllm_storage::db::file_summaries::lookup(&conn.lock().unwrap(), "s1", "h1")
            .unwrap()
            .unwrap();
        assert_ne!(row.status, FileSummaryStatus::Pending);
    }

    #[tokio::test]
    async fn ensure_ready_resolves_when_rows_are_already_done() {
        let conn = summarizer_conn();
        let (tx, _rx) = mpsc::unbounded_channel();
        let summarizer = FileSummarizer::new(
            Arc::clone(&conn),
            libllm_protocol::client::ApiClient::new(
                "http://127.0.0.1:1",
                true,
                libllm_core::config::Auth::None,
            ),
            "summarize this".to_owned(),
            tx,
        );
        {
            let guard = conn.lock().unwrap();
            libllm_storage::db::file_summaries::insert_pending(&guard, "s1", "h1", "a.md").unwrap();
            libllm_storage::db::file_summaries::set_done(&guard, "s1", "h1", "cached").unwrap();
        }

        let file = FileToSummarize {
            basename: "a.md".to_owned(),
            content_hash: "h1".to_owned(),
            body: "body".to_owned(),
        };

        let start = std::time::Instant::now();
        summarizer.ensure_ready("s1", &[file]).await.unwrap();
        assert!(start.elapsed() < std::time::Duration::from_millis(500));
    }
}
