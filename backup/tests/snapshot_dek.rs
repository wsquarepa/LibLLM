use backup::crypto::{resolve_backup_key, unwrap_dek};
use backup::index::{BackupType, FingerprintField, open_index};
use backup::snapshot::create_snapshot;
use libllm::config::BackupConfig;
use tempfile::TempDir;

fn dummy_config() -> BackupConfig {
    BackupConfig {
        enabled: true,
        keep_all_days: 7,
        keep_daily_days: 30,
        keep_weekly_days: 90,
        rebase_threshold_percent: 50,
        rebase_hard_ceiling: 10,
    }
}

fn setup_encrypted_db(data_dir: &std::path::Path, passkey: &str) {
    let salt = libllm::crypto::load_or_create_salt(&data_dir.join(".salt")).unwrap();
    let key = libllm::crypto::derive_key(passkey, &salt).unwrap();
    let db_path = data_dir.join("data.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";\n", &*key.hex()))
        .unwrap();
    conn.execute_batch("PRAGMA journal_mode = WAL;").unwrap();
    conn.execute_batch("CREATE TABLE kv (k TEXT, v TEXT);")
        .unwrap();
}

fn insert_encrypted(data_dir: &std::path::Path, passkey: &str, k: &str, v: &str) {
    let salt = libllm::crypto::load_or_create_salt(&data_dir.join(".salt")).unwrap();
    let key = libllm::crypto::derive_key(passkey, &salt).unwrap();
    let db_path = data_dir.join("data.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";\n", &*key.hex()))
        .unwrap();
    conn.execute(
        "INSERT INTO kv (k, v) VALUES (?1, ?2)",
        rusqlite::params![k, v],
    )
    .unwrap();
}

fn count_kv_encrypted(data_dir: &std::path::Path, passkey: &str) -> i64 {
    let salt = libllm::crypto::load_or_create_salt(&data_dir.join(".salt")).unwrap();
    let key = libllm::crypto::derive_key(passkey, &salt).unwrap();
    let db_path = data_dir.join("data.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";\n", &*key.hex()))
        .unwrap();
    conn.query_row("SELECT COUNT(*) FROM kv", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn new_base_backup_stamps_wrapped_dek_and_fingerprint() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path();
    let passkey = "pw";
    let kek = resolve_backup_key(data_dir, Some(passkey)).unwrap().unwrap();
    setup_encrypted_db(data_dir, passkey);

    create_snapshot(data_dir, Some(passkey), &dummy_config()).unwrap();

    let idx_path = data_dir.join("backups/index.json");
    let index = open_index(&idx_path, Some(&kek)).unwrap();
    let base = index
        .entries
        .iter()
        .find(|e| e.entry_type == BackupType::Base)
        .expect("base exists");
    let wrapped = base.wrapped_dek.as_ref().expect("wrapped DEK on base");
    let _dek = unwrap_dek(wrapped, &kek).expect("DEK unwraps under current KEK");
    assert!(matches!(
        base.kek_fingerprint,
        Some(FingerprintField::Known(ref fp)) if fp.len() == 32
    ));
}

#[test]
fn diff_backup_reuses_chain_dek_and_restore_succeeds() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path();
    let passkey = "pw";
    let kek = resolve_backup_key(data_dir, Some(passkey)).unwrap().unwrap();

    setup_encrypted_db(data_dir, passkey);
    insert_encrypted(data_dir, passkey, "a", "1");
    create_snapshot(data_dir, Some(passkey), &dummy_config()).unwrap();

    insert_encrypted(data_dir, passkey, "b", "2");
    create_snapshot(data_dir, Some(passkey), &dummy_config()).unwrap();

    let idx = open_index(&data_dir.join("backups/index.json"), Some(&kek)).unwrap();
    let diff = idx
        .entries
        .iter()
        .find(|e| e.entry_type == BackupType::Diff)
        .expect("diff present");
    assert!(diff.wrapped_dek.is_none(), "diff must not carry its own DEK");

    let last_id = idx.entries.last().unwrap().id.clone();
    backup::restore::restore_to_point(data_dir, &last_id, Some(passkey), None).unwrap();
    assert_eq!(count_kv_encrypted(data_dir, passkey), 2);
}
