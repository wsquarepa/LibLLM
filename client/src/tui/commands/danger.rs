//! Dispatcher for Danger tab destructive operations.

use anyhow::{Context, Result};

use crate::tui::App;
use crate::tui::types::{DangerOp, DangerSummary, StatusLevel};

pub(in crate::tui) fn spawn_destroy_all(
    bg_tx: tokio::sync::mpsc::Sender<crate::tui::types::BackgroundEvent>,
    data_dir: std::path::PathBuf,
    snapshot_path: std::path::PathBuf,
    summarizer: Option<std::sync::Arc<libllm::files::FileSummarizer>>,
) {
    tokio::spawn(async move {
        if let Some(s) = &summarizer {
            s.shutdown().await;
        }
        let snapshot_path_for_task = snapshot_path.clone();
        let result = tokio::task::spawn_blocking(move || {
            libllm::archive::snapshot_data_dir(&data_dir, &snapshot_path_for_task, "backups")
                .map(|_bytes| crate::tui::types::DangerSummary::SnapshotPath(snapshot_path_for_task))
                .map_err(|e| e.to_string())
        })
        .await
        .unwrap_or_else(|join_err| Err(join_err.to_string()));
        let _ = bg_tx
            .send(crate::tui::types::BackgroundEvent::DangerOpComplete(
                crate::tui::types::DangerOp::DestroyAll,
                result,
            ))
            .await;
    });
}

pub(in crate::tui) fn handle_op_complete(
    app: &mut App,
    op: DangerOp,
    result: std::result::Result<DangerSummary, String>,
) {
    match (op, result) {
        (DangerOp::DestroyAll, Ok(DangerSummary::SnapshotPath(path))) => {
            destroy_all_finalize(app, path);
        }
        (DangerOp::DestroyAll, Err(err)) => {
            app.set_status(
                format!("Snapshot failed: {err}; data dir intact"),
                StatusLevel::Error,
            );
        }
        (op, Ok(summary)) => report_summary(app, op, &summary),
        (_, Err(err)) => {
            app.set_status(format!("Op failed: {err}"), StatusLevel::Error);
        }
    }
}

fn destroy_all_finalize(app: &mut App, snapshot_path: std::path::PathBuf) {
    use std::process;
    let data_dir = libllm::config::data_dir();

    // Drop owned references before deletion — FileSummarizer holds a second DB connection.
    // Other tasks holding Arcs may keep the file alive briefly; acceptable for the destroy path.
    app.file_summarizer = None;
    app.db = None;

    if let Err(err) = std::fs::remove_dir_all(&data_dir) {
        eprintln!(
            "LibLLM: snapshot saved to {} but data dir delete failed: {err}",
            snapshot_path.display()
        );
        process::exit(1);
    }

    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen
    );
    eprintln!(
        "LibLLM data destroyed. Snapshot saved to: {}",
        snapshot_path.display()
    );
    process::exit(0);
}

pub(in crate::tui) fn dispatch_sync(app: &mut App, op: DangerOp) -> Result<DangerSummary> {
    let db = app.db.as_ref().context("database not available")?;
    match op {
        DangerOp::ClearStores => {
            let removed = db
                .clear_dismissed_templates()
                .context("clear_dismissed_templates failed")?;
            Ok(DangerSummary::RowsAffected(removed))
        }
        DangerOp::RegeneratePresets => {
            let summary =
                libllm::preset::regenerate_builtins(&libllm::preset::instruct_presets_dir());
            Ok(DangerSummary::PresetsWritten {
                written: summary.written,
                failed: summary.failed.len(),
            })
        }
        DangerOp::PurgeChats => {
            let n = db.purge_sessions().context("purge_sessions failed")?;
            Ok(DangerSummary::RowsAffected(n))
        }
        DangerOp::PurgeCharacters => {
            let n = db.purge_characters().context("purge_characters failed")?;
            Ok(DangerSummary::RowsAffected(n))
        }
        DangerOp::PurgePersonas => {
            let n = db.purge_personas().context("purge_personas failed")?;
            Ok(DangerSummary::RowsAffected(n))
        }
        DangerOp::PurgeWorldbooks => {
            let n = db.purge_worldbooks().context("purge_worldbooks failed")?;
            Ok(DangerSummary::RowsAffected(n))
        }
        DangerOp::DestroyAll => {
            anyhow::bail!("DestroyAll uses async handler (Task 27)");
        }
    }
}

pub(in crate::tui) fn report_summary(app: &mut App, op: DangerOp, summary: &DangerSummary) {
    let msg = match (op, summary) {
        (DangerOp::ClearStores, DangerSummary::RowsAffected(n)) => {
            format!("Cleared {n} dismissed prompt(s)")
        }
        (DangerOp::RegeneratePresets, DangerSummary::PresetsWritten { written, failed }) => {
            if *failed == 0 {
                format!("Regenerated {written} preset(s)")
            } else {
                format!("Regenerated {written} preset(s); {failed} failed")
            }
        }
        (DangerOp::PurgeChats, DangerSummary::RowsAffected(n)) => {
            format!("Purged {n} chat(s) — restart to reflect changes")
        }
        (DangerOp::PurgeCharacters, DangerSummary::RowsAffected(n)) => {
            format!("Purged {n} character(s)")
        }
        (DangerOp::PurgePersonas, DangerSummary::RowsAffected(n)) => {
            format!("Purged {n} persona(s)")
        }
        (DangerOp::PurgeWorldbooks, DangerSummary::RowsAffected(n)) => {
            format!("Purged {n} worldbook(s)")
        }
        _ => "Operation complete".to_owned(),
    };
    app.set_status(msg, StatusLevel::Info);
}
