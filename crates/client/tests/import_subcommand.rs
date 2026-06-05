#[expect(
    dead_code,
    reason = "each test binary uses a different subset of common helpers"
)]
mod common;

use std::process::Command;

use common::client_bin;
use libllm::db::Database;

#[test]
fn import_persona_from_txt() {
    let dir = common::temp_dir();
    let data_dir = dir.path().join("data");
    let txt_path = dir.path().join("alice.txt");
    std::fs::write(&txt_path, "A curious explorer.").expect("write txt");

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "import",
            "--type",
            "persona",
            txt_path.to_str().unwrap(),
        ])
        .output()
        .expect("spawn client");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db = Database::open(&data_dir.join("data.db"), None).expect("open db");
    let persona = db.load_persona("alice").expect("load alice");
    assert_eq!(persona.name, "alice");
    assert_eq!(persona.persona, "A curious explorer.");
}

#[test]
fn import_character_from_json() {
    let dir = common::temp_dir();
    let data_dir = dir.path().join("data");
    let json_path = dir.path().join("testchar.json");
    std::fs::write(
        &json_path,
        r#"{
  "name": "TestChar",
  "description": "A test character for integration testing.",
  "personality": "Reliable",
  "scenario": "",
  "first_mes": "",
  "mes_example": ""
}"#,
    )
    .expect("write json");

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "import",
            json_path.to_str().unwrap(),
        ])
        .output()
        .expect("spawn client");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db = Database::open(&data_dir.join("data.db"), None).expect("open db");
    let card = db.load_character("testchar").expect("load testchar");
    assert_eq!(card.name, "TestChar");
}

#[test]
fn import_worldbook_from_json() {
    let dir = common::temp_dir();
    let data_dir = dir.path().join("data");
    // File stem "TestBook" becomes the fallback worldbook name when no "name" field is
    // present. The character parser rejects JSON that has no name, so auto-detection
    // falls through to the worldbook parser.
    let json_path = dir.path().join("TestBook.json");
    std::fs::write(
        &json_path,
        r#"{
  "entries": {
    "0": {
      "key": ["lore"],
      "content": "Some lore entry.",
      "disable": false
    }
  }
}"#,
    )
    .expect("write json");

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "import",
            json_path.to_str().unwrap(),
        ])
        .output()
        .expect("spawn client");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db = Database::open(&data_dir.join("data.db"), None).expect("open db");
    let worldbooks = db.list_worldbooks().expect("list worldbooks");
    assert_eq!(worldbooks.len(), 1);
    assert_eq!(worldbooks[0].1, "TestBook");
}

#[test]
fn import_prompt_from_txt() {
    let dir = common::temp_dir();
    let data_dir = dir.path().join("data");
    let txt_path = dir.path().join("myprompt.txt");
    std::fs::write(&txt_path, "You are a concise technical writer.").expect("write txt");

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "import",
            "--type",
            "prompt",
            txt_path.to_str().unwrap(),
        ])
        .output()
        .expect("spawn client");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db = Database::open(&data_dir.join("data.db"), None).expect("open db");
    let prompt = db.load_prompt("myprompt").expect("load myprompt");
    assert_eq!(prompt.content, "You are a concise technical writer.");
}

#[test]
fn import_batch_two_json_files() {
    let dir = common::temp_dir();
    let data_dir = dir.path().join("data");

    let char_path = dir.path().join("batchchar.json");
    std::fs::write(
        &char_path,
        r#"{
  "name": "BatchChar",
  "description": "A batch-imported character.",
  "personality": "",
  "scenario": "",
  "first_mes": "",
  "mes_example": ""
}"#,
    )
    .expect("write char json");

    // No "name" field: character parser rejects it; worldbook parser accepts it.
    let book_path = dir.path().join("batchbook.json");
    std::fs::write(
        &book_path,
        r#"{
  "entries": {
    "0": {
      "key": ["batch"],
      "content": "Batch worldbook entry.",
      "disable": false
    }
  }
}"#,
    )
    .expect("write book json");

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "import",
            char_path.to_str().unwrap(),
            book_path.to_str().unwrap(),
        ])
        .output()
        .expect("spawn client");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db = Database::open(&data_dir.join("data.db"), None).expect("open db");
    let card = db.load_character("batchchar").expect("load batchchar");
    assert_eq!(card.name, "BatchChar");

    let worldbooks = db.list_worldbooks().expect("list worldbooks");
    assert_eq!(worldbooks.len(), 1);
}

#[test]
fn import_rejects_unknown_type() {
    let dir = common::temp_dir();
    let data_dir = dir.path().join("data");
    let txt_path = dir.path().join("file.txt");
    std::fs::write(&txt_path, "content").expect("write txt");

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "import",
            "--type",
            "bogus",
            txt_path.to_str().unwrap(),
        ])
        .output()
        .expect("spawn client");
    assert!(
        !output.status.success(),
        "expected non-zero exit for unknown --type"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("bogus") || stderr.contains("unknown") || stderr.contains("Unknown"),
        "expected error mentioning 'bogus' or 'unknown', got: {stderr}"
    );
}

#[test]
fn import_rejects_txt_without_type() {
    let dir = common::temp_dir();
    let data_dir = dir.path().join("data");
    let txt_path = dir.path().join("mystery.txt");
    std::fs::write(&txt_path, "mystery content").expect("write txt");

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "import",
            txt_path.to_str().unwrap(),
        ])
        .output()
        .expect("spawn client");
    assert!(
        !output.status.success(),
        "expected non-zero exit for .txt without --type"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ambiguous") || stderr.contains("--type") || stderr.contains("type"),
        "expected error about missing type, got: {stderr}"
    );
}

#[test]
fn import_rejects_oversized_persona_file() {
    let dir = common::temp_dir();
    let data_dir = dir.path().join("data");
    let txt_path = dir.path().join("huge.txt");

    let over_limit: u64 = 10 * 1024 * 1024 + 1;
    {
        use std::io::Write as _;
        let mut f = std::fs::File::create(&txt_path).expect("create huge file");
        let chunk = vec![b'x'; 64 * 1024];
        let mut written: u64 = 0;
        while written < over_limit {
            let n = chunk.len().min((over_limit - written) as usize);
            f.write_all(&chunk[..n]).expect("write chunk");
            written += n as u64;
        }
    }

    let output = Command::new(client_bin())
        .args([
            "-d",
            data_dir.to_str().unwrap(),
            "--no-encrypt",
            "import",
            "--type",
            "persona",
            txt_path.to_str().unwrap(),
        ])
        .output()
        .expect("spawn client");
    assert!(
        !output.status.success(),
        "expected non-zero exit for oversized file"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("too large") || stderr.contains("limit"),
        "expected size-limit error, got: {stderr}"
    );
}
