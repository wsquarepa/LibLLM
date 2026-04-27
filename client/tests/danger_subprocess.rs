//! Subprocess test exercising the full Destroy All Data flow end-to-end:
//! spawn the binary against a temp data dir, drive the typed-confirm dialog
//! via the --debug-trigger-destroy-all flag, verify the snapshot exists and
//! the data dir is removed.

#[expect(dead_code, reason = "each test binary uses a different subset of common helpers")]
mod common;

use std::process::Command;

use libllm::db::Database;

#[cfg(not(target_os = "windows"))]
#[test]
fn destroy_all_creates_snapshot_and_removes_data_dir() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let snapshot_dir = tempfile::TempDir::new().unwrap();

    // Seed a database so the data dir is recognized as a valid libllm dir.
    Database::open(&data_dir.path().join("data.db"), None).expect("seed db");

    let output = Command::new(common::client_bin())
        .args([
            "-d",
            data_dir.path().to_str().unwrap(),
            "--no-encrypt",
            "--debug-trigger-destroy-all",
        ])
        .env("TMPDIR", snapshot_dir.path())
        .output()
        .expect("spawn client with debug flag");
    assert!(
        output.status.success(),
        "client exited nonzero: {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let snaps: Vec<_> = std::fs::read_dir(snapshot_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("libllm-"))
        .collect();
    assert_eq!(
        snaps.len(),
        1,
        "expected exactly one snapshot file, got {}",
        snaps.len()
    );

    assert!(!data_dir.path().exists(), "data dir still present");
}
