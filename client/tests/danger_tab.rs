//! Integration tests for Danger tab synchronous dispatch operations.
//! Drives the libllm-level functions directly; the full TUI flow is exercised
//! in `danger_subprocess.rs` for Destroy All Data.

#[expect(dead_code, reason = "each test binary uses a different subset of common helpers")]
mod common;

use libllm::db::migrations::run_migrations;
use rusqlite::Connection;

fn fresh_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn).unwrap();
    conn
}

#[test]
fn clear_stores_empties_dismissed_template_prompts() {
    let conn = fresh_conn();
    libllm::db::record_template_dismissal(&conn, "h1").unwrap();
    libllm::db::record_template_dismissal(&conn, "h2").unwrap();
    let removed = libllm::db::clear_dismissed_templates(&conn).unwrap();
    assert_eq!(removed, 2);
    assert!(!libllm::db::is_template_dismissed(&conn, "h1").unwrap());
}

#[test]
fn regenerate_presets_restores_overwritten_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let chatml_path = dir.path().join("ChatML.json");
    std::fs::write(&chatml_path, "garbage").unwrap();
    let summary = libllm::preset::regenerate_builtins(dir.path());
    assert!(summary.failed.is_empty());
    let restored = std::fs::read_to_string(&chatml_path).unwrap();
    assert!(restored.contains("im_start") || restored.contains("ChatML"));
}

#[test]
fn purge_table_clears_target_table() {
    let conn = fresh_conn();
    conn.execute(
        "INSERT INTO characters (slug, name, created_at, updated_at) \
         VALUES ('t', 'test', '2024-01-01', '2024-01-01')",
        [],
    )
    .unwrap();
    let before: i64 = conn
        .query_row("SELECT COUNT(*) FROM characters", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, 1);

    let affected = conn.execute("DELETE FROM characters", []).unwrap();
    assert_eq!(affected, 1);

    let after: i64 = conn
        .query_row("SELECT COUNT(*) FROM characters", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after, 0);
}
