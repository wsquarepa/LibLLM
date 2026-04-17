#[expect(dead_code, reason = "each test binary uses a different subset of common helpers")]
mod common;

use std::io::Write as _;
use std::process::{Command, Stdio};

use common::client_bin;
use libllm::db::Database;
use libllm::persona::PersonaFile;

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
fn import_rejects_schema_version_mismatch() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    let db_path = data_dir.join("data.db");
    seed_plain_db(&db_path);

    let bad = dir.path().join("bad.db");
    {
        let bad_conn = rusqlite::Connection::open(&bad).unwrap();
        bad_conn
            .execute_batch(
                "CREATE TABLE schema_version (version INTEGER NOT NULL); \
                 INSERT INTO schema_version (version) VALUES (999);",
            )
            .unwrap();
    }

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "import",
            "--yes",
            bad.to_str().unwrap(),
        ])
        .output()
        .expect("spawn client");

    assert_eq!(output.status.code(), Some(3), "expected exit code 3");

    let after = Database::open(&db_path, None).expect("open data.db");
    let personas = after.list_personas().expect("list");
    assert_eq!(personas.len(), 1, "import must not have touched the live db");
}

#[test]
fn import_round_trip_unencrypted() {
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
        .expect("spawn dump");
    assert!(status.success());

    {
        let plain = Database::open(&dump_path, None).expect("open dump");
        plain
            .delete_persona("alice")
            .expect("delete alice in dump");
        plain
            .insert_persona(
                "carol",
                &PersonaFile {
                    name: "Carol".to_owned(),
                    persona: "new".to_owned(),
                },
            )
            .expect("insert carol in dump");
    }

    let status = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "import",
            "--yes",
            dump_path.to_str().unwrap(),
        ])
        .status()
        .expect("spawn import");
    assert!(status.success(), "import exit status: {status:?}");

    let after = Database::open(&db_path, None).expect("reopen data.db");
    let personas = after.list_personas().expect("list");
    assert_eq!(personas, vec![("carol".to_owned(), "Carol".to_owned())]);
}

#[test]
fn import_failure_leaves_original_intact() {
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
        .expect("spawn dump");
    assert!(status.success());

    let original_bytes = std::fs::read(&db_path).expect("read pre-import data.db");

    let blocker = data_dir.join("data.import.tmp");
    std::fs::create_dir(&blocker).expect("create blocker dir");

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "import",
            "--yes",
            dump_path.to_str().unwrap(),
        ])
        .output()
        .expect("spawn import");
    assert!(!output.status.success(), "import should have failed");

    let after_bytes = std::fs::read(&db_path).expect("read post-import data.db");
    assert_eq!(original_bytes, after_bytes, "data.db must be byte-identical");
}

#[test]
fn import_creates_backup_first_encrypted() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    let db_path = data_dir.join("data.db");
    let key = common::test_key(data_dir);
    {
        let db = Database::open(&db_path, Some(&key)).expect("open enc");
        db.insert_persona(
            "alice",
            &PersonaFile {
                name: "Alice".to_owned(),
                persona: "curious".to_owned(),
            },
        )
        .unwrap();
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
        .expect("spawn dump");
    assert!(status.success());

    let backups_before: Vec<_> = std::fs::read_dir(data_dir.join("backups"))
        .map(|rd| rd.flatten().collect())
        .unwrap_or_default();

    let status = Command::new(client_bin())
        .env("LIBLLM_PASSKEY", "test-passkey")
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "db",
            "import",
            "--yes",
            dump_path.to_str().unwrap(),
        ])
        .status()
        .expect("spawn import");
    assert!(status.success(), "import exit status: {status:?}");

    let backups_after: Vec<_> = std::fs::read_dir(data_dir.join("backups"))
        .expect("backups dir exists after import")
        .flatten()
        .collect();
    assert!(
        backups_after.len() > backups_before.len(),
        "import must create a new backup entry"
    );
}

#[test]
fn wal_liveness_refuses_dump_and_import_when_db_is_held() {
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
        .expect("spawn initial dump");
    assert!(status.success());

    // Hold an exclusive write transaction on the encrypted db so the liveness
    // probe sees SQLITE_BUSY.
    let holder = rusqlite::Connection::open(&db_path).expect("holder open");
    holder
        .execute_batch("BEGIN IMMEDIATE;")
        .expect("hold write lock");

    let dump_again = dir.path().join("dump2.db");
    let dump_output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "dump",
            dump_again.to_str().unwrap(),
        ])
        .output()
        .expect("spawn second dump");
    assert_eq!(dump_output.status.code(), Some(4), "dump should exit 4 on busy");

    let import_output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "import",
            "--yes",
            dump_path.to_str().unwrap(),
        ])
        .output()
        .expect("spawn import");
    assert_eq!(import_output.status.code(), Some(4), "import should exit 4 on busy");

    drop(holder);
}

#[test]
fn sql_read_only_rejects_update() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    seed_plain_db(&data_dir.join("data.db"));

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "sql",
            "UPDATE personas SET name = 'X' WHERE slug = 'alice';",
        ])
        .output()
        .expect("spawn client");
    assert!(!output.status.success(), "expected failure exit code");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("readonly") || stderr.contains("read-only"),
        "expected readonly error, got: {stderr}"
    );
}

#[test]
fn sql_write_allows_update() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    let db_path = data_dir.join("data.db");
    seed_plain_db(&db_path);

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "sql",
            "--write",
            "UPDATE personas SET name = 'AliceX' WHERE slug = 'alice';",
        ])
        .output()
        .expect("spawn client");
    assert!(
        output.status.success(),
        "exit status: {:?}, stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let db = Database::open(&db_path, None).expect("open db");
    let loaded = db.load_persona("alice").expect("load alice");
    assert_eq!(loaded.name, "AliceX");
}

#[test]
fn sql_rejects_multi_statement() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    seed_plain_db(&data_dir.join("data.db"));

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "sql",
            "SELECT 1; SELECT 2;",
        ])
        .output()
        .expect("spawn client");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("single statement"),
        "expected single-statement error, got: {stderr}"
    );
}

#[test]
fn sql_emits_csv_format() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    seed_plain_db(&data_dir.join("data.db"));

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "sql",
            "--format",
            "csv",
            "SELECT slug, name FROM personas ORDER BY slug;",
        ])
        .output()
        .expect("spawn client");
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, "slug,name\nalice,Alice\n");
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

#[test]
fn dump_handles_output_path_with_tmp_extension() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    let db_path = data_dir.join("data.db");
    seed_plain_db(&db_path);

    // Output path that ends in `.tmp` — would collide with the internal
    // tmp-file name if the dump computed it via `with_extension`.
    let out = dir.path().join("backup.tmp");

    let status = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "dump",
            out.to_str().unwrap(),
        ])
        .status()
        .expect("spawn client");
    assert!(status.success(), "dump exit status: {status:?}");

    let dumped = libllm::db::Database::open(&out, None).expect("open dump");
    let personas = dumped.list_personas().expect("list");
    assert_eq!(personas.len(), 1);
    assert_eq!(personas[0].0, "alice");
}

#[test]
fn shell_runs_scripted_select() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    seed_plain_db(&data_dir.join("data.db"));

    let mut child = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "shell",
            "--private",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn shell");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(stdin, ".mode pipe").unwrap();
        writeln!(stdin, ".headers off").unwrap();
        writeln!(stdin, "SELECT slug FROM personas;").unwrap();
        writeln!(stdin, ".quit").unwrap();
    }
    let output = child.wait_with_output().expect("wait");
    assert!(output.status.success(), "exit: {:?} stderr: {}", output.status, String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("alice"), "stdout: {stdout}");
}

#[test]
fn shell_history_respects_ignorespace() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    seed_plain_db(&data_dir.join("data.db"));

    let mut child = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "shell",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn shell");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "SELECT 1;").unwrap();
        writeln!(stdin, " SELECT 2;").unwrap();
        writeln!(stdin, ".quit").unwrap();
    }
    let _ = child.wait_with_output().expect("wait");

    let history = std::fs::read_to_string(data_dir.join(".db_shell_history"))
        .expect("history file should exist");
    assert!(history.contains("SELECT 1;"));
    assert!(!history.contains("SELECT 2;"), "leading-space line must be excluded");
}

#[test]
fn shell_private_mode_writes_no_history() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    seed_plain_db(&data_dir.join("data.db"));

    let mut child = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "shell",
            "--private",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn shell");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "SELECT 1;").unwrap();
        writeln!(stdin, ".quit").unwrap();
    }
    let _ = child.wait_with_output().expect("wait");

    let history_path = data_dir.join(".db_shell_history");
    assert!(!history_path.exists(), "private mode must not write a history file");
}

#[test]
fn shell_rejects_query_only_pragma_in_read_only_mode() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    seed_plain_db(&data_dir.join("data.db"));

    let mut child = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "shell",
            "--private",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn shell");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "PRAGMA query_only = OFF;").unwrap();
        writeln!(stdin, ".quit").unwrap();
    }
    let output = child.wait_with_output().expect("wait");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("query_only") && stderr.contains("read-only"),
        "expected query_only rejection message, got: {stderr}"
    );
}

#[test]
fn sql_rejects_query_only_pragma_in_read_only_mode() {
    let dir = common::temp_dir();
    let data_dir = dir.path();
    seed_plain_db(&data_dir.join("data.db"));

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "db",
            "sql",
            "PRAGMA query_only = OFF;",
        ])
        .output()
        .expect("spawn client");
    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("query_only") && stderr.contains("read-only"),
        "expected query_only rejection message, got: {stderr}"
    );
}
