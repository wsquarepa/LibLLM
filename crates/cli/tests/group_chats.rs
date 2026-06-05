#[expect(
    dead_code,
    reason = "each test binary uses a different subset of common helpers"
)]
mod common;

use std::process::Command;

use common::{client_bin, import_card, import_persona, temp_dir};
use libllm::group_chat::{CharacterAttachment, ChatMode, decide_next_speaker};
use libllm::session::Session;
use libllm_tui::dialogs::chat_settings::{ChatSettingsDialog, Row};
use libllm_tui::match_next_candidates;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rusqlite::Connection;

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
    let db = libllm::db::Database::open(&db_path, None).unwrap();

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
fn directed_mode_decide_next_speaker_returns_none() {
    let cs = vec![CharacterAttachment {
        slug: "alice".into(),
        talkativeness: 1.0,
        action_points: 0.0,
        spoke_this_round: false,
    }];
    let mut rng = StdRng::seed_from_u64(0);
    assert!(
        decide_next_speaker(&cs, ChatMode::Directed, &mut rng, None).is_none(),
        "directed mode must never auto-select a speaker"
    );
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
fn next_autocomplete_substring_picks_alice() {
    let cast = vec![
        ("alice-slug", "Alice the Wise"),
        ("bob-slug", "Bob the Knight"),
    ];
    let matches = match_next_candidates("ali wi", &cast);
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

fn seed_v8_file(path: &std::path::Path) {
    let conn = Connection::open(path).expect("open v8 file");
    conn.execute_batch(
        "CREATE TABLE schema_version (version INTEGER NOT NULL);

         CREATE TABLE sessions (
             id TEXT PRIMARY KEY NOT NULL,
             display_name TEXT,
             model TEXT,
             template TEXT,
             system_prompt TEXT,
             character TEXT,
             persona TEXT,
             head_id INTEGER,
             created_at TEXT NOT NULL,
             updated_at TEXT NOT NULL
         );

         CREATE TABLE session_worldbooks (
             session_id TEXT NOT NULL,
             worldbook_slug TEXT NOT NULL,
             PRIMARY KEY (session_id, worldbook_slug),
             FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
         );

         CREATE TABLE messages (
             id INTEGER NOT NULL,
             session_id TEXT NOT NULL,
             parent_id INTEGER,
             preferred_child_id INTEGER,
             role TEXT NOT NULL,
             content TEXT NOT NULL,
             timestamp TEXT NOT NULL,
             thought_seconds INTEGER,
             speaker_slug TEXT,
             pre_turn_action_points TEXT,
             PRIMARY KEY (session_id, id),
             FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
         );

         CREATE INDEX idx_messages_session ON messages(session_id);

         CREATE TABLE characters (
             slug TEXT PRIMARY KEY NOT NULL,
             name TEXT NOT NULL,
             description TEXT,
             personality TEXT,
             scenario TEXT,
             first_mes TEXT,
             mes_example TEXT,
             system_prompt TEXT,
             post_history_instructions TEXT,
             alternate_greetings TEXT,
             author_note TEXT,
             author_note_depth INTEGER NOT NULL DEFAULT 4,
             author_note_at_top INTEGER NOT NULL DEFAULT 0,
             created_at TEXT NOT NULL,
             updated_at TEXT NOT NULL
         );

         CREATE TABLE worldbooks (
             slug TEXT PRIMARY KEY NOT NULL,
             name TEXT NOT NULL,
             entries TEXT NOT NULL,
             created_at TEXT NOT NULL,
             updated_at TEXT NOT NULL
         );

         CREATE TABLE system_prompts (
             slug TEXT PRIMARY KEY NOT NULL,
             name TEXT NOT NULL,
             content TEXT NOT NULL,
             builtin INTEGER NOT NULL,
             created_at TEXT NOT NULL,
             updated_at TEXT NOT NULL
         );

         CREATE TABLE personas (
             slug TEXT PRIMARY KEY NOT NULL,
             name TEXT NOT NULL,
             persona TEXT NOT NULL,
             created_at TEXT NOT NULL,
             updated_at TEXT NOT NULL
         );

         CREATE TABLE file_summaries (
             session_id   TEXT NOT NULL,
             content_hash TEXT NOT NULL,
             basename     TEXT NOT NULL,
             summary      TEXT NOT NULL DEFAULT '',
             status       TEXT NOT NULL,
             created_at   TEXT NOT NULL,
             updated_at   TEXT NOT NULL,
             PRIMARY KEY (session_id, content_hash),
             FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
         );

         CREATE INDEX idx_file_summaries_status ON file_summaries(status);

         CREATE TABLE dismissed_template_prompts (
             template_hash TEXT PRIMARY KEY,
             dismissed_at INTEGER NOT NULL
         );

         ALTER TABLE sessions ADD COLUMN chat_policy TEXT NOT NULL DEFAULT 'round_robin';
         ALTER TABLE sessions ADD COLUMN card_assembly TEXT NOT NULL DEFAULT 'join_cards';

         CREATE TABLE session_characters (
             session_id     TEXT NOT NULL,
             slug           TEXT NOT NULL,
             attach_index   INTEGER NOT NULL,
             talkativeness  REAL NOT NULL DEFAULT 0.5,
             action_points  REAL NOT NULL DEFAULT 0.0,
             PRIMARY KEY (session_id, slug),
             FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
         );
         CREATE INDEX idx_session_characters_session
             ON session_characters(session_id, attach_index);

         ALTER TABLE sessions ADD COLUMN author_note TEXT;
         ALTER TABLE sessions ADD COLUMN author_note_depth INTEGER NOT NULL DEFAULT 4;
         ALTER TABLE sessions ADD COLUMN author_note_at_top INTEGER NOT NULL DEFAULT 0;

         CREATE VIRTUAL TABLE messages_fts USING fts5(
             content,
             content='messages',
             content_rowid='rowid',
             tokenize='unicode61 remove_diacritics 2'
         );

         CREATE TRIGGER messages_fts_ai AFTER INSERT ON messages BEGIN
             INSERT INTO messages_fts(rowid, content) VALUES (new.rowid, new.content);
         END;

         CREATE TRIGGER messages_fts_ad AFTER DELETE ON messages BEGIN
             INSERT INTO messages_fts(messages_fts, rowid, content)
             VALUES('delete', old.rowid, old.content);
         END;

         CREATE TRIGGER messages_fts_au AFTER UPDATE OF content ON messages BEGIN
             INSERT INTO messages_fts(messages_fts, rowid, content)
             VALUES('delete', old.rowid, old.content);
             INSERT INTO messages_fts(rowid, content)
             VALUES (new.rowid, new.content);
         END;

         INSERT INTO schema_version (version) VALUES (8);",
    )
    .expect("seed v8 schema");
}

#[test]
fn legacy_v8_group_session_migrates_with_synthesized_scenario() {
    let dir = temp_dir();
    let db_path = dir.path().join("v8.db");

    seed_v8_file(&db_path);

    {
        let conn = Connection::open(&db_path).expect("open v8 conn for data");
        conn.execute_batch(
            "INSERT INTO characters (slug, name, description, personality, scenario, first_mes, mes_example, system_prompt, post_history_instructions, created_at, updated_at) \
             VALUES ('alice', 'Alice', '', '', 'Alice is hunting.', '', '', '', '', 'now', 'now');
             INSERT INTO characters (slug, name, description, personality, scenario, first_mes, mes_example, system_prompt, post_history_instructions, created_at, updated_at) \
             VALUES ('bob', 'Bob', '', '', 'Bob is brewing.', '', '', '', '', 'now', 'now');
             INSERT INTO sessions (id, display_name, created_at, updated_at, head_id, character, chat_policy, card_assembly) \
             VALUES ('g1', 'group', 'now', 'now', NULL, NULL, 'weighted_random', 'join_cards');
             INSERT INTO session_characters (session_id, slug, attach_index, talkativeness, action_points) \
             VALUES ('g1', 'alice', 0, 0.5, 0.0);
             INSERT INTO session_characters (session_id, slug, attach_index, talkativeness, action_points) \
             VALUES ('g1', 'bob', 1, 0.5, 0.0);",
        )
        .expect("insert v8 data");
    }

    let db = libllm::db::Database::open(&db_path, None).expect("open with v9 migration");
    let loaded = db.load_session("g1").expect("load migrated session");

    let expected = "[Scenario for Alice]\nAlice is hunting.\n[Scenario for Bob]\nBob is brewing.";
    assert_eq!(
        loaded.scenario.as_deref(),
        Some(expected),
        "v9 migration must synthesize scenario from attached character cards"
    );
}

#[test]
fn scenario_editor_cancel_does_not_write_provisional_to_session() {
    use libllm::group_chat::CharacterAttachment;
    use libllm::session::Session;
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
    use libllm::group_chat::CharacterAttachment;
    use libllm::session::Session;
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
