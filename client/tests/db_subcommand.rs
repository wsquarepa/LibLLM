#[expect(dead_code, reason = "each test binary uses a different subset of common helpers")]
mod common;

use std::process::Command;

use libllm::db::Database;
use libllm::persona::PersonaFile;

fn client_bin() -> std::path::PathBuf {
    let target = env!("CARGO_BIN_EXE_client");
    std::path::PathBuf::from(target)
}

fn seed_plain_db(path: &std::path::Path) {
    let db = Database::open(path, None).expect("open plain db");
    db.insert_persona(
        "alice",
        &PersonaFile {
            name: "Alice".to_owned(),
            persona: "curious".to_owned(),
        },
    )
    .expect("insert alice");
}

#[test]
fn dump_round_trip_unencrypted() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    let db_path = data_dir.join("data.db");
    seed_plain_db(&db_path);

    let dump_path = dir.path().join("dump.db");

    let status = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "dump",
            dump_path.to_str().unwrap(),
        ])
        .status()
        .expect("spawn client");
    assert!(status.success(), "dump exit status: {status:?}");

    let dumped = Database::open(&dump_path, None).expect("open dump");
    let personas = dumped.list_personas().expect("list");
    assert_eq!(personas.len(), 1);
    assert_eq!(personas[0].0, "alice");
}

#[test]
fn dump_round_trip_encrypted() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    let db_path = data_dir.join("data.db");
    let key = common::test_key(data_dir);
    {
        let db = Database::open(&db_path, Some(&key)).expect("open enc");
        db.insert_persona(
            "bob",
            &PersonaFile {
                name: "Bob".to_owned(),
                persona: "wise".to_owned(),
            },
        )
        .expect("insert bob");
    }

    let dump_path = dir.path().join("dump.db");

    let status = Command::new(client_bin())
        .env("LIBLLM_PASSKEY", "test-passkey")
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "db",
            "dump",
            dump_path.to_str().unwrap(),
        ])
        .status()
        .expect("spawn client");
    assert!(status.success(), "dump exit status: {status:?}");

    let dumped = Database::open(&dump_path, None).expect("open dump as plain");
    let personas = dumped.list_personas().expect("list");
    assert_eq!(personas, vec![("bob".to_owned(), "Bob".to_owned())]);
}
