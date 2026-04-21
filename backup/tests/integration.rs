use std::path::Path;
use std::time::Duration;

use backup::BackupConfig;
use backup::index::{self, BackupType};
use backup::restore;
use backup::snapshot;
use backup::verify;

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

fn setup_encrypted_db(dir: &Path, passkey: &str) -> std::path::PathBuf {
    let salt = libllm::crypto::load_or_create_salt(&dir.join(".salt")).unwrap();
    let key = libllm::crypto::derive_key(passkey, &salt).unwrap();
    let db_path = dir.join("data.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";\n", &*key.hex()))
        .unwrap();
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

fn insert_note_encrypted(db_path: &Path, content: &str, passkey: &str) {
    let salt_path = db_path.parent().unwrap().join(".salt");
    let salt = libllm::crypto::load_or_create_salt(&salt_path).unwrap();
    let key = libllm::crypto::derive_key(passkey, &salt).unwrap();
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";\n", &*key.hex()))
        .unwrap();
    conn.execute("INSERT INTO notes (content) VALUES (?1)", [content])
        .unwrap();
}

fn count_notes(db_path: &Path) -> i64 {
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.query_row("SELECT count(*) FROM notes", [], |row| row.get(0))
        .unwrap()
}

fn count_notes_encrypted(db_path: &Path, passkey: &str) -> i64 {
    let salt_path = db_path.parent().unwrap().join(".salt");
    let salt = libllm::crypto::load_or_create_salt(&salt_path).unwrap();
    let key = libllm::crypto::derive_key(passkey, &salt).unwrap();
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";\n", &*key.hex()))
        .unwrap();
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

    let verify_result = verify::verify_chain(dir.path(), None, None, false).unwrap();
    assert!(
        verify_result.errors.is_empty(),
        "expected no verify errors, got: {:?}",
        verify_result.errors
    );

    insert_note(&db_path, "fourth note");
    assert_eq!(count_notes(&db_path), 4);

    let second_id = idx.entries[1].id.clone();
    restore::restore_to_point(dir.path(), &second_id, None, None).unwrap();

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
    conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";\n", &*db_key.hex()))
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

    let verify_result = verify::verify_chain(dir.path(), Some("test-passkey"), None, false).unwrap();
    assert!(
        verify_result.errors.is_empty(),
        "expected no verify errors, got: {:?}",
        verify_result.errors
    );
}

#[test]
fn encrypted_backup_restore_cycle() {
    let dir = tempfile::TempDir::new().unwrap();
    let config = BackupConfig::default();
    let db_path = setup_encrypted_db(dir.path(), "test-passkey");

    snapshot::create_snapshot(dir.path(), Some("test-passkey"), &config).unwrap();

    insert_note_encrypted(&db_path, "second note", "test-passkey");
    std::thread::sleep(Duration::from_secs(1));
    snapshot::create_snapshot(dir.path(), Some("test-passkey"), &config).unwrap();

    insert_note_encrypted(&db_path, "third note", "test-passkey");
    assert_eq!(count_notes_encrypted(&db_path, "test-passkey"), 3);

    let index_path = dir.path().join("backups").join("index.json");
    let idx = index::load_index(&index_path).unwrap();
    let diff_id = idx
        .entries
        .iter()
        .find(|e| e.entry_type == BackupType::Diff)
        .unwrap()
        .id
        .clone();

    restore::restore_to_point(dir.path(), &diff_id, Some("test-passkey"), None).unwrap();

    assert_eq!(count_notes_encrypted(&db_path, "test-passkey"), 2);

    // Verify the restored DB is still encrypted: opening without a key must fail to read data.
    let conn_no_key = rusqlite::Connection::open(&db_path).unwrap();
    let unkeyed_result: rusqlite::Result<i64> =
        conn_no_key.query_row("SELECT count(*) FROM notes", [], |row| row.get(0));
    assert!(
        unkeyed_result.is_err(),
        "expected failure when reading encrypted DB without a key"
    );

    // Opening with the correct key must succeed.
    assert_eq!(count_notes_encrypted(&db_path, "test-passkey"), 2);
}

#[test]
fn restore_with_wrong_passkey_fails() {
    let dir = tempfile::TempDir::new().unwrap();
    let config = BackupConfig::default();
    let db_path = setup_encrypted_db(dir.path(), "correct-passkey");

    snapshot::create_snapshot(dir.path(), Some("correct-passkey"), &config).unwrap();

    let index_path = dir.path().join("backups").join("index.json");
    let idx = index::load_index(&index_path).unwrap();
    let id = idx.entries[0].id.clone();

    let result = restore::restore_to_point(dir.path(), &id, Some("wrong-passkey"), None);
    assert!(
        result.is_err(),
        "expected restore with wrong passkey to fail"
    );

    // data.db must still be readable with the correct key and have 1 row.
    assert_eq!(count_notes_encrypted(&db_path, "correct-passkey"), 1);
}

#[test]
fn restore_to_nonexistent_id_fails() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = setup_db(dir.path());
    let config = BackupConfig::default();

    snapshot::create_snapshot(dir.path(), None, &config).unwrap();

    let result = restore::restore_to_point(dir.path(), "nonexistent-id", None, None);
    assert!(
        result.is_err(),
        "expected restore to nonexistent id to fail"
    );

    // data.db must still be intact.
    assert_eq!(count_notes(&db_path), 1);
}

#[test]
fn corrupted_backup_file_fails_restore() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = setup_db(dir.path());
    let config = BackupConfig::default();

    snapshot::create_snapshot(dir.path(), None, &config).unwrap();

    insert_note(&db_path, "second note");
    std::thread::sleep(Duration::from_secs(1));
    snapshot::create_snapshot(dir.path(), None, &config).unwrap();

    // 2 rows in the live DB before attempting restore.
    assert_eq!(count_notes(&db_path), 2);

    let backups_dir = dir.path().join("backups");
    let index_path = backups_dir.join("index.json");
    let idx = index::load_index(&index_path).unwrap();

    let base_entry = idx
        .entries
        .iter()
        .find(|e| e.entry_type == BackupType::Base)
        .unwrap();
    let base_file = backups_dir.join(&base_entry.filename);
    std::fs::write(&base_file, b"this is not a valid backup file").unwrap();

    let diff_id = idx
        .entries
        .iter()
        .find(|e| e.entry_type == BackupType::Diff)
        .unwrap()
        .id
        .clone();

    let result = restore::restore_to_point(dir.path(), &diff_id, None, None);
    assert!(
        result.is_err(),
        "expected restore with corrupted base to fail"
    );

    // data.db must still reflect its pre-restore state (2 rows).
    assert_eq!(count_notes(&db_path), 2);
}

#[test]
fn verify_full_replay_detects_corruption() {
    let dir = tempfile::TempDir::new().unwrap();
    setup_db(dir.path());
    let config = BackupConfig::default();

    snapshot::create_snapshot(dir.path(), None, &config).unwrap();

    let backups_dir = dir.path().join("backups");
    let index_path = backups_dir.join("index.json");
    let idx = index::load_index(&index_path).unwrap();
    let bak_file = backups_dir.join(&idx.entries[0].filename);
    std::fs::write(&bak_file, b"garbage that cannot be decompressed").unwrap();

    let result = verify::verify_chain(dir.path(), None, None, true).unwrap();
    assert!(
        !result.errors.is_empty(),
        "expected verify errors for corrupted backup file"
    );
}

#[test]
fn verify_full_replay_encrypted_chain_with_passkey_passes() {
    let dir = tempfile::TempDir::new().unwrap();
    let config = BackupConfig::default();
    let db_path = setup_encrypted_db(dir.path(), "test-passkey");

    snapshot::create_snapshot(dir.path(), Some("test-passkey"), &config).unwrap();

    std::thread::sleep(Duration::from_secs(1));
    insert_note_encrypted(&db_path, "second note", "test-passkey");
    snapshot::create_snapshot(dir.path(), Some("test-passkey"), &config).unwrap();

    std::thread::sleep(Duration::from_secs(1));
    insert_note_encrypted(&db_path, "third note", "test-passkey");
    snapshot::create_snapshot(dir.path(), Some("test-passkey"), &config).unwrap();

    let index_path = dir.path().join("backups").join("index.json");
    let idx = index::load_index(&index_path).unwrap();
    assert_eq!(idx.entries.len(), 3);
    assert!(idx.entries.iter().all(|e| e.encrypted));
    assert_eq!(idx.entries[0].entry_type, BackupType::Base);
    assert_eq!(idx.entries[1].entry_type, BackupType::Diff);
    assert_eq!(idx.entries[2].entry_type, BackupType::Diff);

    let result = verify::verify_chain(dir.path(), Some("test-passkey"), None, true).unwrap();
    assert!(
        result.errors.is_empty(),
        "expected no errors on encrypted full-replay with correct passkey, got: {:?}",
        result.errors
    );
    assert_eq!(result.checked_count, 3);
}

#[test]
fn verify_full_replay_encrypted_chain_without_passkey_reports_single_clear_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let config = BackupConfig::default();
    let db_path = setup_encrypted_db(dir.path(), "test-passkey");

    snapshot::create_snapshot(dir.path(), Some("test-passkey"), &config).unwrap();

    std::thread::sleep(Duration::from_secs(1));
    insert_note_encrypted(&db_path, "second note", "test-passkey");
    snapshot::create_snapshot(dir.path(), Some("test-passkey"), &config).unwrap();

    // Full replay without a passkey must not feed ciphertext into zstd; it must report
    // one actionable error naming the passkey requirement rather than N zstd failures.
    let result = verify::verify_chain(dir.path(), None, None, true).unwrap();
    assert_eq!(
        result.errors.len(),
        1,
        "expected exactly one error, got: {:?}",
        result.errors
    );
    let message = &result.errors[0];
    assert!(
        message.contains("passkey"),
        "expected error to mention passkey, got: {message}"
    );
    assert!(
        !message.contains("Unknown frame descriptor"),
        "must not surface raw zstd error: {message}"
    );
}

#[test]
fn restore_encrypted_chain_without_passkey_fails_with_clear_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let config = BackupConfig::default();
    setup_encrypted_db(dir.path(), "test-passkey");

    snapshot::create_snapshot(dir.path(), Some("test-passkey"), &config).unwrap();

    let index_path = dir.path().join("backups").join("index.json");
    let idx = index::load_index(&index_path).unwrap();
    let id = idx.entries[0].id.clone();

    let err = restore::restore_to_point(dir.path(), &id, None, None)
        .expect_err("restore without passkey on encrypted chain must fail");
    let message = format!("{err:#}");
    assert!(
        message.contains("encrypted") && message.contains("passkey"),
        "expected explicit encryption/passkey error, got: {message}"
    );
    assert!(
        !message.contains("Unknown frame descriptor"),
        "must not surface raw zstd error: {message}"
    );
}

#[test]
fn rebase_threshold_triggers_new_base() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = setup_db(dir.path());
    // A very low threshold forces a new base whenever the diff exceeds 10% of the base size.
    let config = BackupConfig {
        rebase_threshold_percent: 10,
        ..BackupConfig::default()
    };

    snapshot::create_snapshot(dir.path(), None, &config).unwrap();

    // Drastically change the DB: drop the existing table and create a new one with many rows.
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("DROP TABLE notes;").unwrap();
        conn.execute_batch(
            "CREATE TABLE big_data (id INTEGER PRIMARY KEY AUTOINCREMENT, payload TEXT NOT NULL);",
        )
        .unwrap();
        for i in 0..200 {
            conn.execute(
                "INSERT INTO big_data (payload) VALUES (?1)",
                [format!(
                    "row-payload-{i}-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                )],
            )
            .unwrap();
        }
    }

    std::thread::sleep(Duration::from_secs(1));
    snapshot::create_snapshot(dir.path(), None, &config).unwrap();

    let index_path = dir.path().join("backups").join("index.json");
    let idx = index::load_index(&index_path).unwrap();
    assert_eq!(idx.entries.len(), 2);
    assert_eq!(
        idx.entries[1].entry_type,
        BackupType::Base,
        "expected second snapshot to be a Base due to rebase threshold being exceeded"
    );
}
