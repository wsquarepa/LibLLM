#[expect(dead_code, reason = "each test binary uses a different subset of common helpers")]
mod common;

use std::path::PathBuf;

use client::import::{detect_import_type, handle_import_command, import_single_file, ImportType};
use libllm::db::Database;

#[test]
fn import_character_from_json() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let db = Database::open(&db_path, None).expect("open db");

    let char_path = dir.path().join("testchar.json");
    common::write_json_file(
        &char_path,
        r#"{"name":"TestChar","description":"A test character"}"#,
    );

    import_single_file(&char_path, &ImportType::Character, &db)
        .expect("import character should succeed");

    let characters = db.list_characters().expect("list characters");
    assert!(
        characters.iter().any(|(_, name)| name == "TestChar"),
        "imported character should appear in db"
    );
}

#[test]
fn import_worldbook_from_json() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let db = Database::open(&db_path, None).expect("open db");

    let wb_path = dir.path().join("mytome.json");
    common::write_json_file(
        &wb_path,
        r#"{"name":"MyTome","entries":[]}"#,
    );

    import_single_file(&wb_path, &ImportType::Worldbook, &db)
        .expect("import worldbook should succeed");

    let worldbooks = db.list_worldbooks().expect("list worldbooks");
    assert!(
        worldbooks.iter().any(|(_, name)| name == "MyTome"),
        "imported worldbook should appear in db"
    );
}

#[test]
fn import_persona_from_txt() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let db = Database::open(&db_path, None).expect("open db");

    let persona_path = dir.path().join("mypersona.txt");
    std::fs::write(&persona_path, "I am a friendly assistant persona.")
        .expect("write persona file");

    import_single_file(&persona_path, &ImportType::Persona, &db)
        .expect("import persona should succeed");

    let personas = db.list_personas().expect("list personas");
    assert!(
        personas.iter().any(|(_, name)| name == "mypersona"),
        "imported persona should appear in db"
    );
}

#[test]
fn import_system_prompt_from_txt() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let db = Database::open(&db_path, None).expect("open db");

    let prompt_path = dir.path().join("mysysprompt.txt");
    std::fs::write(&prompt_path, "You are a helpful assistant.").expect("write prompt file");

    import_single_file(&prompt_path, &ImportType::SystemPrompt, &db)
        .expect("import system prompt should succeed");

    let prompts = db.list_prompts().expect("list prompts");
    assert!(
        prompts.iter().any(|p| p.name == "mysysprompt"),
        "imported system prompt should appear in db"
    );
}

#[test]
fn import_batch_mixed_files() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let db = Database::open(&db_path, None).expect("open db");

    let char_path = dir.path().join("batchchar.json");
    common::write_json_file(
        &char_path,
        r#"{"name":"BatchChar","description":"Batch imported character"}"#,
    );

    let files: Vec<PathBuf> = vec![char_path];
    handle_import_command(&files, None, &db).expect("batch import should succeed");

    let characters = db.list_characters().expect("list characters");
    assert!(
        characters.iter().any(|(_, name)| name == "BatchChar"),
        "batch-imported character should appear in db"
    );
}

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
    assert!(result.is_err(), "batch import with missing file should fail");
}

#[test]
fn detect_import_type_json_sniffs_character() {
    let dir = common::temp_dir();
    let char_path = dir.path().join("sniffme.json");
    common::write_json_file(
        &char_path,
        r#"{"name":"SniffChar","description":"Auto-detected character"}"#,
    );

    let import_type = detect_import_type(&char_path, None).expect("detect type should succeed");
    assert!(
        matches!(import_type, ImportType::Character),
        "character JSON should be detected as Character"
    );
}

#[test]
fn detect_import_type_txt_without_kind_errors() {
    let dir = common::temp_dir();
    let txt_path = dir.path().join("ambiguous.txt");
    std::fs::write(&txt_path, "some content").expect("write txt file");

    let result = detect_import_type(&txt_path, None);
    assert!(
        result.is_err(),
        ".txt without explicit kind should return an error"
    );
}
