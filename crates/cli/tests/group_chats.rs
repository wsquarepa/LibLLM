#[expect(
    dead_code,
    reason = "each test binary uses a different subset of common helpers"
)]
mod common;

use std::process::Command;

use common::{client_bin, import_card, import_persona, temp_dir};
use libllm_core::group_chat::{CharacterAttachment, ChatMode};
use libllm_core::session::Session;
use libllm_tui::dialogs::chat_settings::{ChatSettingsDialog, Row};
use libllm_tui::match_next_candidates;

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
    let chars: Vec<(String, String)> = (0..9).map(|i| (format!("c{i}"), format!("C{i}"))).collect();
    let pairs: Vec<(&str, &str)> = chars
        .iter()
        .map(|(s, n)| (s.as_str(), n.as_str()))
        .collect();
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
    let db = libllm_storage::db::Database::open(&db_path, None).unwrap();

    // Insert a solo session using only the legacy columns. No session_characters row is
    // written — this emulates a session written by an older binary before migrations ran.
    db.execute_statement(
        "INSERT INTO sessions (id, character, created_at, updated_at) \
         VALUES ('legacy-solo', 'alice', 'now', 'now')",
    )
    .unwrap();

    // Confirm no session_characters row so load_session exercises the synthesis path.
    db.execute_statement("DELETE FROM session_characters WHERE session_id = 'legacy-solo'")
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

#[test]
fn next_picker_resolves_multi_word_name() {
    let cast = vec![
        ("alice-slug", "Alice the Wise"),
        ("bob-slug", "Bob the Knight"),
    ];
    let matches = match_next_candidates("Alice the Wise", &cast);
    assert_eq!(matches, vec!["Alice the Wise"]);
}

#[test]
fn chat_settings_solo_shows_only_scenario_row() {
    let session = Session {
        characters: vec![CharacterAttachment {
            slug: "alice".into(),
            talkativeness: 1.0,
            action_points: 0.0,
            spoke_this_round: false,
        }],
        ..Session::default()
    };
    let dialog = ChatSettingsDialog::for_session(&session);
    assert_eq!(dialog.rows.len(), 2);
    assert!(matches!(dialog.rows[0], Row::Scenario));
    assert!(matches!(dialog.rows[1], Row::Buttons));
}

#[test]
fn chat_settings_group_shows_mode_and_sliders() {
    let session = Session {
        characters: vec![
            CharacterAttachment {
                slug: "alice".into(),
                talkativeness: 0.5,
                action_points: 0.0,
                spoke_this_round: false,
            },
            CharacterAttachment {
                slug: "bob".into(),
                talkativeness: 0.5,
                action_points: 0.0,
                spoke_this_round: false,
            },
        ],
        ..Session::default()
    };
    let dialog = ChatSettingsDialog::for_session(&session);
    assert_eq!(dialog.rows.len(), 5);
    assert!(matches!(dialog.rows[0], Row::Scenario));
    assert!(matches!(dialog.rows[1], Row::Mode));
    assert!(matches!(dialog.rows[2], Row::Talkativeness { index: 0 }));
    assert!(matches!(dialog.rows[3], Row::Talkativeness { index: 1 }));
    assert!(matches!(dialog.rows[4], Row::Buttons));
}

#[test]
fn scenario_editor_cancel_does_not_write_provisional_to_session() {
    use libllm_core::group_chat::CharacterAttachment;
    use libllm_core::session::Session;
    use libllm_tui::dialogs::chat_settings::ChatSettingsDialog;

    let mut session = Session {
        scenario: Some("original".to_owned()),
        characters: vec![CharacterAttachment::new("alice".to_owned())],
        ..Session::default()
    };
    let mut dialog = ChatSettingsDialog::for_session(&session);

    // Load a provisional value into the dialog (simulates the scenario editor closing
    // with typed text before the user cancels Chat Settings).
    dialog.set_provisional_scenario(Some("edited but canceled".to_owned()));

    // On Cancel the snapshot is restored; the provisional value must NOT be written
    // to session.scenario at any point.
    dialog.restore_snapshot(&mut session);

    assert_eq!(
        session.scenario.as_deref(),
        Some("original"),
        "Cancel must restore the original scenario, not the provisional edit"
    );
}

#[test]
fn scenario_editor_save_commits_provisional_to_session() {
    use libllm_core::group_chat::CharacterAttachment;
    use libllm_core::session::Session;
    use libllm_tui::dialogs::chat_settings::ChatSettingsDialog;

    let mut session = Session {
        scenario: Some("original".to_owned()),
        characters: vec![CharacterAttachment::new("alice".to_owned())],
        ..Session::default()
    };
    let mut dialog = ChatSettingsDialog::for_session(&session);

    // Simulate the scenario editor writing a provisional value into the dialog,
    // then the parent Save committing it to session.scenario.
    dialog.set_provisional_scenario(Some("new scenario".to_owned()));
    dialog.commit_provisional_scenario(&mut session);

    assert_eq!(
        session.scenario.as_deref(),
        Some("new scenario"),
        "Save must commit the provisional scenario to the session"
    );
}

#[test]
fn cleared_provisional_scenario_save_commits_none() {
    use libllm_core::group_chat::CharacterAttachment;
    use libllm_core::session::Session;
    use libllm_tui::dialogs::chat_settings::{ChatSettingsDialog, ProvisionalScenario};

    let mut session = Session {
        scenario: Some("original".to_owned()),
        characters: vec![CharacterAttachment::new("alice".to_owned())],
        ..Session::default()
    };
    let mut dialog = ChatSettingsDialog::for_session(&session);

    dialog.set_provisional_scenario(None);
    assert!(matches!(
        dialog.provisional_scenario_state(),
        ProvisionalScenario::Cleared
    ));

    dialog.commit_provisional_scenario(&mut session);
    assert_eq!(
        session.scenario, None,
        "Save after clear must commit None, not leave the stale session scenario"
    );
}

#[test]
fn absent_provisional_scenario_keeps_session_on_save() {
    use libllm_core::group_chat::CharacterAttachment;
    use libllm_core::session::Session;
    use libllm_tui::dialogs::chat_settings::{ChatSettingsDialog, ProvisionalScenario};

    let mut session = Session {
        scenario: Some("original".to_owned()),
        characters: vec![CharacterAttachment::new("alice".to_owned())],
        ..Session::default()
    };
    let mut dialog = ChatSettingsDialog::for_session(&session);

    assert!(matches!(
        dialog.provisional_scenario_state(),
        ProvisionalScenario::Absent
    ));

    dialog.commit_provisional_scenario(&mut session);
    assert_eq!(
        session.scenario.as_deref(),
        Some("original"),
        "Save without opening the editor must leave session.scenario untouched"
    );
}
