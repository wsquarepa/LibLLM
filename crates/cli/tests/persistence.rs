#[expect(
    dead_code,
    reason = "each test binary uses a different subset of common helpers"
)]
mod common;

use libllm_core::crypto;
use libllm_core::group_chat::{CharacterAttachment, ChatMode};
use libllm_core::session::{MessageTree, Role, Session};
use libllm_storage::db::Database;

#[test]
fn session_encrypted_database_round_trip() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let key = common::test_key(dir.path());

    let session = common::linear_session(vec![
        common::user_msg("secret question"),
        common::assistant_msg("secret answer"),
    ]);
    {
        let mut db = Database::open(&db_path, Some(&key)).expect("open encrypted db");
        db.insert_session("enc-1", &session).expect("insert failed");
    }
    {
        let db = Database::open(&db_path, Some(&key)).expect("reopen encrypted db");
        let loaded = db.load_session("enc-1").expect("load failed");
        let messages = loaded.tree.branch_path();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "secret question");
        assert_eq!(messages[1].content, "secret answer");
    }
}

#[test]
fn session_wrong_key_rejected() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let key_a = common::test_key(dir.path());

    let salt_b_path = dir.path().join(".salt_b");
    let salt_b = crypto::load_or_create_salt(&salt_b_path).expect("salt_b");
    let key_b = crypto::derive_key("different-passkey", &salt_b).expect("derive key_b");

    {
        let mut db = Database::open(&db_path, Some(&key_a)).expect("open with key_a");
        let session = common::linear_session(vec![common::user_msg("private")]);
        db.insert_session("wrong-key-1", &session).expect("insert");
    }

    let result = Database::open(&db_path, Some(&key_b));
    assert!(result.is_err(), "opening with wrong key should fail");
}

#[test]
fn session_empty_round_trip() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let mut db = Database::open(&db_path, None).expect("open db");

    let session = common::linear_session(vec![]);
    db.insert_session("empty-1", &session)
        .expect("insert empty");
    let loaded = db.load_session("empty-1").expect("load empty");

    assert_eq!(loaded.tree.branch_path().len(), 0);
    assert!(loaded.tree.head().is_none());
}

#[test]
fn session_duplicate_subtree() {
    let mut tree = MessageTree::new();
    let n0 = tree.push(None, common::user_msg("first"));
    let n1 = tree.push(Some(n0), common::assistant_msg("second"));
    let _n2 = tree.push(Some(n1), common::user_msg("third"));

    let copy_root = tree.duplicate_subtree(n1).expect("duplicate_subtree");

    assert_ne!(copy_root, n1);

    tree.set_message_content(copy_root, "modified copy".to_string());
    let original = tree.node(n1).expect("original node");
    assert_eq!(original.message.content, "second");

    let copy = tree.node(copy_root).expect("copy node");
    assert_eq!(copy.message.content, "modified copy");
}

#[test]
fn session_set_message_content() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let mut db = Database::open(&db_path, None).expect("open db");

    let mut session = common::linear_session(vec![
        common::user_msg("original"),
        common::assistant_msg("reply"),
    ]);

    let head_id = session.tree.head().expect("has head");
    let updated = session
        .tree
        .set_message_content(head_id, "edited reply".to_string());
    assert!(updated, "set_message_content should return true");

    db.insert_session("edit-1", &session).expect("insert");
    let loaded = db.load_session("edit-1").expect("load");

    let messages = loaded.tree.branch_path();
    assert_eq!(messages[1].content, "edited reply");
}

#[test]
fn role_summary_round_trip() {
    let role = Role::Summary;
    let serialized = role.to_string();
    assert_eq!(serialized, "summary");
    let deserialized: Role = serialized.parse().unwrap();
    assert_eq!(deserialized, role);
}

#[test]
fn worldbook_rename_insert_before_delete_preserves_old_on_failure() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let db = Database::open(&db_path, None).expect("open db");

    let original = common::worldbook("old", vec![common::worldbook_entry(vec!["key"], "lore")]);
    db.insert_worldbook("old", &original)
        .expect("insert original");

    let conflicting = common::worldbook("conflict", vec![]);
    db.insert_worldbook("conflict", &conflicting)
        .expect("insert conflicting");

    let new_wb = common::worldbook("old-renamed", vec![]);
    let insert_result = db.insert_worldbook("conflict", &new_wb);

    assert!(
        insert_result.is_err(),
        "inserting a duplicate slug must fail"
    );

    let still_there = db.load_worldbook("old");
    assert!(
        still_there.is_ok(),
        "original worldbook must still exist when the insert fails before the delete"
    );
}

#[test]
fn group_session_scenario_and_chat_mode_round_trip() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let mut db = Database::open(&db_path, None).expect("open db");

    let session = Session {
        characters: vec![
            CharacterAttachment {
                slug: "alice".to_owned(),
                talkativeness: 0.4,
                action_points: 0.0,
                spoke_this_round: false,
            },
            CharacterAttachment {
                slug: "bob".to_owned(),
                talkativeness: 0.6,
                action_points: 0.0,
                spoke_this_round: false,
            },
        ],
        chat_mode: ChatMode::ActionValue,
        scenario: Some("a tavern".to_owned()),
        ..Default::default()
    };

    db.insert_session("grp-scenario", &session)
        .expect("insert session with scenario");
    let loaded = db
        .load_session("grp-scenario")
        .expect("load session with scenario");

    assert_eq!(loaded.scenario.as_deref(), Some("a tavern"));
    assert!(matches!(loaded.chat_mode, ChatMode::ActionValue));
    assert!((loaded.characters[0].talkativeness - 0.4_f32).abs() < 1e-6);
    assert!((loaded.characters[1].talkativeness - 0.6_f32).abs() < 1e-6);
}

#[test]
fn session_scenario_is_independent_of_card_scenario_after_save() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let db = Database::open(&db_path, None).expect("open db");

    let card = common::simple_character("Alice", "An adventurer.");
    db.insert_character("alice", &card).expect("insert card");

    let mut session = Session {
        scenario: Some("In a tavern.".to_owned()),
        characters: vec![CharacterAttachment {
            slug: "alice".to_owned(),
            talkativeness: 1.0,
            action_points: 0.0,
            spoke_this_round: false,
        }],
        ..Default::default()
    };
    {
        let mut db2 = Database::open(&db_path, None).expect("reopen for insert");
        db2.insert_session("solo-scenario", &session)
            .expect("insert session");
    }

    session.scenario = Some("Changed in session.".to_owned());
    {
        let mut db3 = Database::open(&db_path, None).expect("reopen for save");
        db3.save_session("solo-scenario", &session)
            .expect("save session");
    }

    let db4 = Database::open(&db_path, None).expect("reopen for verify");
    let reloaded_card = db4.load_character("alice").expect("load card");
    assert_eq!(
        reloaded_card.scenario.as_str(),
        "",
        "modifying the session scenario must not mutate the character card"
    );
    let reloaded_session = db4
        .load_session("solo-scenario")
        .expect("load session after mutation");
    assert_eq!(
        reloaded_session.scenario.as_deref(),
        Some("Changed in session."),
        "session scenario must reflect the saved change"
    );
}
