#[expect(dead_code, reason = "each test binary uses a different subset of common helpers")]
mod common;

use std::process::Command;

use common::{client_bin, import_card, import_persona, temp_dir};

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
