#[expect(
    dead_code,
    reason = "each test binary uses a different subset of common helpers"
)]
mod common;

use std::path::PathBuf;

use libllm_cli::import::handle_import_command;
use libllm_storage::db::Database;

#[test]
fn import_batch_partial_failure() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let db = Database::open(&db_path, None).expect("open db");

    let char_path = dir.path().join("goodchar.json");
    common::write_json_file(
        &char_path,
        r#"{"name":"GoodChar","description":"Valid character"}"#,
    );

    let missing_path = dir.path().join("nonexistent.json");

    let files: Vec<PathBuf> = vec![char_path, missing_path];
    let result = handle_import_command(&files, None, &db);
    assert!(
        result.is_err(),
        "batch import with missing file should fail"
    );
}
