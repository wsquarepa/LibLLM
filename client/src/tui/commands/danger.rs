//! Dispatcher for Danger tab destructive operations.

use anyhow::{Context, Result};

use crate::tui::App;
use crate::tui::types::{DangerOp, DangerSummary, StatusLevel};

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
