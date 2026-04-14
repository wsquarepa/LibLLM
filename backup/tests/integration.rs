use std::path::Path;
use std::time::Duration;

use backup::index::{self, BackupType};
use backup::restore;
use backup::snapshot;
use backup::verify;
use backup::BackupConfig;

fn setup_db(dir: &Path) -> std::path::PathBuf {
    let db_path = dir.join("data.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE notes (id INTEGER PRIMARY KEY AUTOINCREMENT, content TEXT NOT NULL);",
    )
    .unwrap();
    conn.execute("INSERT INTO notes (content) VALUES (?1)", ["first note"])
        .unwrap();
    drop(conn);
    db_path
}

fn insert_note(db_path: &Path, content: &str) {
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute("INSERT INTO notes (content) VALUES (?1)", [content])
        .unwrap();
}

fn count_notes(db_path: &Path) -> i64 {
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.query_row("SELECT count(*) FROM notes", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn full_backup_restore_cycle_unencrypted() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = setup_db(dir.path());
    let config = BackupConfig::default();

    snapshot::create_snapshot(dir.path(), None, &config).unwrap();

    std::thread::sleep(Duration::from_secs(1));
    insert_note(&db_path, "second note");
    snapshot::create_snapshot(dir.path(), None, &config).unwrap();

    std::thread::sleep(Duration::from_secs(1));
    insert_note(&db_path, "third note");
    snapshot::create_snapshot(dir.path(), None, &config).unwrap();

    let index_path = dir.path().join("backups").join("index.json");
    let idx = index::load_index(&index_path).unwrap();
    assert_eq!(idx.entries.len(), 3);
    assert_eq!(idx.entries[0].entry_type, BackupType::Base);
    assert_eq!(idx.entries[1].entry_type, BackupType::Diff);
    assert_eq!(idx.entries[2].entry_type, BackupType::Diff);

    let verify_result = verify::verify_chain(dir.path(), None, false).unwrap();
    assert!(
        verify_result.errors.is_empty(),
        "expected no verify errors, got: {:?}",
        verify_result.errors
    );

    insert_note(&db_path, "fourth note");
    assert_eq!(count_notes(&db_path), 4);

    let second_id = idx.entries[1].id.clone();
    restore::restore_to_point(dir.path(), &second_id, None).unwrap();

    assert_eq!(count_notes(&db_path), 2);
}

#[test]
fn full_backup_restore_cycle_encrypted() {
    let dir = tempfile::TempDir::new().unwrap();
    let config = BackupConfig::default();

    let salt = libllm::crypto::load_or_create_salt(&dir.path().join(".salt")).unwrap();
    let db_key = libllm::crypto::derive_key("test-passkey", &salt).unwrap();

    let db_path = dir.path().join("data.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";\n", db_key.hex()))
        .unwrap();
    conn.execute_batch(
        "CREATE TABLE notes (id INTEGER PRIMARY KEY AUTOINCREMENT, content TEXT NOT NULL);",
    )
    .unwrap();
    conn.execute("INSERT INTO notes (content) VALUES (?1)", ["first note"])
        .unwrap();
    drop(conn);

    snapshot::create_snapshot(dir.path(), Some("test-passkey"), &config).unwrap();

    let index_path = dir.path().join("backups").join("index.json");
    let idx = index::load_index(&index_path).unwrap();
    assert_eq!(idx.entries.len(), 1);
    assert!(idx.entries[0].encrypted, "expected entry to be encrypted");

    let verify_result =
        verify::verify_chain(dir.path(), Some("test-passkey"), false).unwrap();
    assert!(
        verify_result.errors.is_empty(),
        "expected no verify errors, got: {:?}",
        verify_result.errors
    );
}
