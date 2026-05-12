#[expect(dead_code, reason = "each test binary uses a different subset of common helpers")]
mod common;

use std::process::Command;

use common::{client_bin, import_card, import_persona, temp_dir};
use libllm::group_chat::ChatMode;

fn workspace_with(chars: &[(&str, &str)], persona: Option<(&str, &str)>) -> tempfile::TempDir {
    let ws = temp_dir();
    for (slug, name) in chars {
        import_card(ws.path(), slug, name);
    }
    if let Some((slug, name)) = persona {
        import_persona(ws.path(), slug, name);
    }
    ws
}

#[test]
fn solo_session_unchanged() {
    let ws = workspace_with(&[("alice", "Alice")], Some(("me", "Trav")));
    let out = Command::new(client_bin())
        .args(["-d", ws.path().to_str().unwrap(), "--no-encrypt"])
        .args(["-c", "alice", "-p", "me"])
        .args(["--help"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn group_without_persona_fails() {
    let ws = workspace_with(&[("alice", "Alice"), ("bob", "Bob")], None);
    let out = Command::new(client_bin())
        .args(["-d", ws.path().to_str().unwrap(), "--no-encrypt"])
        .args(["-c", "alice", "-c", "bob"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("persona") || stderr.contains("required") || stderr.contains("requires"),
        "expected persona-required error in stderr: {stderr}",
    );
}

#[test]
fn talkativeness_with_unknown_slug_fails() {
    let ws = workspace_with(&[("alice", "Alice"), ("bob", "Bob")], Some(("me", "Trav")));
    let out = Command::new(client_bin())
        .args(["-d", ws.path().to_str().unwrap(), "--no-encrypt"])
        .args(["-c", "alice", "-c", "bob", "-p", "me"])
        .args(["--talkativeness", "ghost=0.5"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("ghost"),
        "expected 'ghost' in stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn over_cap_characters_fail() {
    let chars: Vec<(String, String)> = (0..9)
        .map(|i| (format!("c{i}"), format!("C{i}")))
        .collect();
    let pairs: Vec<(&str, &str)> = chars.iter().map(|(s, n)| (s.as_str(), n.as_str())).collect();
    let ws = workspace_with(&pairs, Some(("me", "Trav")));

    let mut cmd = Command::new(client_bin());
    cmd.args(["-d", ws.path().to_str().unwrap(), "--no-encrypt"])
        .args(["-p", "me"]);
    for (slug, _) in &chars {
        cmd.args(["-c", slug.as_str()]);
    }
    let out = cmd.output().unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("limited to"),
        "expected 'limited to' in stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn legacy_v4_solo_session_loads_with_v5_backfill() {
    let dir = temp_dir();
    let db_path = dir.path().join("sessions.db");

    // Open via Database::open, which runs all migrations and creates the current schema.
    let db = libllm::db::Database::open(&db_path, None).unwrap();

    // Insert a solo session using only the legacy columns. No session_characters row is
    // written — this emulates a session written by an older binary before migrations ran.
    db.execute_statement(
        "INSERT INTO sessions (id, character, created_at, updated_at) \
         VALUES ('legacy-solo', 'alice', 'now', 'now')",
    )
    .unwrap();

    // Confirm no session_characters row so load_session exercises the synthesis path.
    db.execute_statement(
        "DELETE FROM session_characters WHERE session_id = 'legacy-solo'",
    )
    .unwrap();

    let loaded = db.load_session("legacy-solo").unwrap();

    assert_eq!(
        loaded.characters.len(),
        1,
        "synthesis from sessions.character should produce one attachment"
    );
    assert_eq!(loaded.characters[0].slug, "alice");
    assert!(
        (loaded.characters[0].talkativeness - 1.0).abs() < 1e-6,
        "synthesized attachment must use talkativeness=1.0"
    );
    // sessions.character is preserved as the legacy mirror column.
    assert_eq!(loaded.character.as_deref(), Some("alice"));
    // The column was originally added in v5 with DEFAULT 'round_robin'; v9 renames it but
    // preserves the default, so legacy sessions without an explicit value load as RoundRobin.
    assert!(matches!(loaded.chat_mode, ChatMode::RoundRobin));
}
