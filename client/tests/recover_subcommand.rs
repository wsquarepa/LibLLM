#[expect(
    dead_code,
    reason = "each test binary uses a different subset of common helpers"
)]
mod common;

use std::process::Command;

use backup::index::load_index;
use backup::snapshot::create_snapshot;
use common::client_bin;
use libllm::config::BackupConfig;
use libllm::db::Database;
use libllm::persona::PersonaFile;

fn seed_db(path: &std::path::Path) {
    let db = Database::open(path, None).expect("open plain db");
    db.insert_persona(
        "alice",
        &PersonaFile {
            name: "alice".to_owned(),
            persona: "curious".to_owned(),
        },
    )
    .expect("insert alice");
}

#[test]
fn recover_list_without_backups() {
    let dir = common::temp_dir();
    let data_dir = dir.path();

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "recover",
            "list",
        ])
        .output()
        .expect("spawn client");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No backup points found"),
        "expected empty-list message, got: {stdout}"
    );
}

#[test]
fn recover_list_after_backup() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    let db_path = data_dir.join("data.db");
    seed_db(&db_path);
    create_snapshot(data_dir, None, &BackupConfig::default()).expect("create snapshot");

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "recover",
            "list",
        ])
        .output()
        .expect("spawn client");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.is_empty(),
        "expected non-empty stdout listing backups, got empty"
    );
    // Backup IDs have the format YYYYMMDDTHHmmssZ
    assert!(
        stdout.contains('T') && stdout.contains('Z'),
        "expected backup ID in stdout, got: {stdout}"
    );
}

#[test]
fn recover_verify_passes_clean_chain() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    let db_path = data_dir.join("data.db");
    seed_db(&db_path);

    let config = BackupConfig::default();
    create_snapshot(data_dir, None, &config).expect("first snapshot");
    create_snapshot(data_dir, None, &config).expect("second snapshot");

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "recover",
            "verify",
        ])
        .output()
        .expect("spawn client");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn recover_restore_round_trip() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    let db_path = data_dir.join("data.db");
    seed_db(&db_path);

    let config = BackupConfig::default();
    create_snapshot(data_dir, None, &config).expect("snapshot after seeding alice");

    let index_path = data_dir.join("backups").join("index.json");
    let index = load_index(&index_path).expect("load index");
    let snapshot_id = index.entries[0].id.clone();

    {
        let db = Database::open(&db_path, None).expect("open db for mutation");
        db.insert_persona(
            "bob",
            &PersonaFile {
                name: "bob".to_owned(),
                persona: "wise".to_owned(),
            },
        )
        .expect("insert bob");
        db.delete_persona("alice").expect("delete alice");
    }

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "recover",
            "restore",
            &snapshot_id,
            "--yes",
        ])
        .output()
        .expect("spawn client");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db = Database::open(&db_path, None).expect("reopen db after restore");
    let personas = db.list_personas().expect("list personas");
    assert_eq!(
        personas,
        vec![("alice".to_owned(), "alice".to_owned())],
        "restored db must match original state"
    );
}

#[test]
fn recover_rebuild_index_after_index_deletion() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    let db_path = data_dir.join("data.db");
    seed_db(&db_path);

    let config = BackupConfig::default();
    create_snapshot(data_dir, None, &config).expect("first snapshot");
    create_snapshot(data_dir, None, &config).expect("second snapshot");

    let index_path = data_dir.join("backups").join("index.json");
    std::fs::remove_file(&index_path).expect("remove index.json");

    let rebuild_output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "recover",
            "rebuild-index",
        ])
        .output()
        .expect("spawn client");
    assert!(
        rebuild_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&rebuild_output.stderr)
    );

    let list_output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "recover",
            "list",
        ])
        .output()
        .expect("spawn client");
    assert!(
        list_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&list_output.stderr)
    );
    let stdout = String::from_utf8_lossy(&list_output.stdout);
    assert!(
        stdout.contains('T') && stdout.contains('Z'),
        "expected backup entries in list after rebuild, got: {stdout}"
    );
}

#[test]
fn recover_rebuild_index_preserves_diff_restore_points() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    let db_path = data_dir.join("data.db");
    seed_db(&db_path);

    let config = BackupConfig::default();
    create_snapshot(data_dir, None, &config).expect("base snapshot");

    {
        let db = Database::open(&db_path, None).expect("open db for diff mutation");
        db.insert_persona(
            "bob",
            &PersonaFile {
                name: "bob".to_owned(),
                persona: "wise".to_owned(),
            },
        )
        .expect("insert bob");
    }

    std::thread::sleep(std::time::Duration::from_secs(1));
    create_snapshot(data_dir, None, &config).expect("diff snapshot");

    let index_path = data_dir.join("backups").join("index.json");
    let original_index = load_index(&index_path).expect("load original index");
    let diff_id = original_index
        .entries
        .iter()
        .find(|entry| entry.entry_type == backup::index::BackupType::Diff)
        .expect("diff entry")
        .id
        .clone();

    std::fs::remove_file(&index_path).expect("remove index.json");

    let rebuild_output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "recover",
            "rebuild-index",
        ])
        .output()
        .expect("spawn client");
    assert!(
        rebuild_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&rebuild_output.stderr)
    );

    {
        let db = Database::open(&db_path, None).expect("open db for post-diff mutation");
        db.insert_persona(
            "carol",
            &PersonaFile {
                name: "carol".to_owned(),
                persona: "calm".to_owned(),
            },
        )
        .expect("insert carol");
    }

    let restore_output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "recover",
            "restore",
            &diff_id,
            "--yes",
        ])
        .output()
        .expect("spawn client");
    assert!(
        restore_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&restore_output.stderr)
    );

    let db = Database::open(&db_path, None).expect("reopen db after restore");
    let personas = db.list_personas().expect("list personas");
    assert_eq!(
        personas,
        vec![
            ("alice".to_owned(), "alice".to_owned()),
            ("bob".to_owned(), "bob".to_owned()),
        ],
        "restore after rebuild must preserve diff snapshot state"
    );
}

#[test]
fn recover_refuses_legacy_dir_with_data_db_and_no_salt() {
    let dir = common::temp_dir();
    let data_dir = dir.path();

    let sessions_dir = data_dir.join("sessions");
    std::fs::create_dir(&sessions_dir).unwrap();
    std::fs::write(sessions_dir.join("session.json"), "{}").unwrap();
    std::fs::write(data_dir.join("data.db"), b"pretend encrypted database").unwrap();

    let output = Command::new(client_bin())
        .args(["-d", data_dir.to_str().unwrap(), "recover", "list"])
        .output()
        .expect("spawn client");
    assert!(
        !output.status.success(),
        "expected recover to refuse a legacy dir whose data.db is unpaired with .salt; \
         stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(".salt") && stderr.contains("data.db"),
        "expected error to mention missing .salt alongside data.db, got: {stderr}"
    );
}

#[test]
fn recover_list_labels_archived_chain() {
    let dir_a = common::temp_dir();
    let data_dir_a = dir_a.path();
    let salt_a =
        libllm::crypto::load_or_create_salt(&data_dir_a.join(".salt")).expect("create dir_a salt");
    let key_a = libllm::crypto::derive_key("pw-a", &salt_a).expect("derive dir_a key");
    let db_path_a = data_dir_a.join("data.db");
    {
        let db = Database::open(&db_path_a, Some(&key_a)).expect("open encrypted dir_a db");
        db.insert_persona(
            "alice",
            &libllm::persona::PersonaFile {
                name: "alice".to_owned(),
                persona: "curious".to_owned(),
            },
        )
        .expect("insert alice into dir_a db");
    }
    create_snapshot(data_dir_a, Some("pw-a"), &BackupConfig::default()).expect("snapshot dir_a");

    let dir_b = common::temp_dir();
    let data_dir_b = dir_b.path();
    let salt_b =
        libllm::crypto::load_or_create_salt(&data_dir_b.join(".salt")).expect("create dir_b salt");
    let key_b = libllm::crypto::derive_key("pw-b", &salt_b).expect("derive dir_b key");
    {
        let _db = Database::open(&data_dir_b.join("data.db"), Some(&key_b))
            .expect("create dir_b encrypted db");
    }

    let backups_b = data_dir_b.join("backups");
    std::fs::create_dir_all(&backups_b).expect("create dir_b backups");
    let backups_a = data_dir_a.join("backups");
    for entry in std::fs::read_dir(&backups_a).expect("read dir_a backups") {
        let src = entry.expect("read dir_a entry").path();
        let dst = backups_b.join(src.file_name().expect("src file name"));
        std::fs::copy(&src, &dst).expect("copy backup file");
    }

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir_b.to_str().unwrap(),
            "--passkey",
            "pw-b",
            "recover",
            "list",
        ])
        .output()
        .expect("spawn client");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let archived_pos = stdout
        .find("archived ")
        .unwrap_or_else(|| panic!("expected 'archived ' in stdout, got: {stdout}"));
    let after_archived = &stdout[archived_pos + "archived ".len()..];
    let hex_run: usize = after_archived
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .count();
    assert!(
        hex_run >= 8,
        "expected 8+ hex chars after 'archived ', got {hex_run} in: {stdout}"
    );
}

#[test]
fn recover_no_subcommand_non_tty_prints_help() {
    let dir = common::temp_dir();
    let data_dir = dir.path();

    let output = Command::new(client_bin())
        .args(["-d", data_dir.to_str().unwrap(), "--no-encrypt", "recover"])
        .output()
        .expect("spawn client");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("list")
            || combined.contains("verify")
            || combined.contains("restore")
            || combined.contains("Usage"),
        "expected help output, got: {combined}"
    );
}
