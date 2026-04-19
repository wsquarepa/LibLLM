#[expect(
    dead_code,
    reason = "each test binary uses a different subset of common helpers"
)]
mod common;

use libllm::crypto;
use libllm::db::Database;
use libllm::session::{MessageTree, Role, Session};

#[test]
fn session_database_round_trip() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let mut db = Database::open(&db_path, None).expect("open db");

    let session = common::linear_session(vec![
        common::user_msg("hello"),
        common::assistant_msg("hi there"),
        common::user_msg("how are you?"),
    ]);
    db.insert_session("plain-1", &session)
        .expect("insert failed");
    let loaded = db.load_session("plain-1").expect("load failed");

    let messages = loaded.tree.branch_path();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, Role::User);
    assert_eq!(messages[0].content, "hello");
    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[1].content, "hi there");
    assert_eq!(messages[2].role, Role::User);
    assert_eq!(messages[2].content, "how are you?");
}

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
fn session_branching() {
    let mut tree = MessageTree::new();

    let root = tree.push(None, common::user_msg("root"));
    let a1 = tree.push(Some(root), common::assistant_msg("branch A"));
    let _a2 = tree.push(Some(a1), common::user_msg("continue A"));

    let b1 = tree.push(Some(root), common::assistant_msg("branch B"));

    let path = tree.branch_path();
    assert_eq!(path.len(), 2);
    assert_eq!(path[0].content, "root");
    assert_eq!(path[1].content, "branch B");

    let root_node = tree.node(root).expect("root node");
    assert_eq!(root_node.children.len(), 2);

    let siblings = tree.siblings_of(b1);
    assert!(siblings.contains(&a1));
    assert!(siblings.contains(&b1));
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
fn session_metadata_fields_survive_round_trip() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let mut db = Database::open(&db_path, None).expect("open db");

    let session = Session {
        tree: MessageTree::new(),
        model: Some("llama-3".to_string()),
        template: Some("chatml".to_string()),
        system_prompt: Some("Be helpful.".to_string()),
        character: Some("TestChar".to_string()),
        worldbooks: vec!["lore-a".to_string(), "lore-b".to_string()],
        persona: Some("Alice".to_string()),
    };
    db.insert_session("meta-1", &session).expect("insert meta");
    let loaded = db.load_session("meta-1").expect("load meta");

    assert_eq!(loaded.model.as_deref(), Some("llama-3"));
    assert_eq!(loaded.template.as_deref(), Some("chatml"));
    assert_eq!(loaded.system_prompt.as_deref(), Some("Be helpful."));
    assert_eq!(loaded.character.as_deref(), Some("TestChar"));
    assert_eq!(loaded.worldbooks, vec!["lore-a", "lore-b"]);
    assert_eq!(loaded.persona.as_deref(), Some("Alice"));
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
fn worldbook_rename_insert_then_delete_completes_cleanly() {
    let dir = common::temp_dir();
    let db_path = dir.path().join("data.db");
    let db = Database::open(&db_path, None).expect("open db");

    let original = common::worldbook("old", vec![common::worldbook_entry(vec!["key"], "lore")]);
    db.insert_worldbook("old", &original)
        .expect("insert original");

    let renamed = common::worldbook(
        "new-name",
        vec![common::worldbook_entry(vec!["key"], "lore")],
    );
    db.insert_worldbook("new-name", &renamed)
        .expect("insert with new slug succeeds");
    db.delete_worldbook("old")
        .expect("delete old slug succeeds");

    assert!(db.load_worldbook("old").is_err(), "old slug must be gone");
    assert!(db.load_worldbook("new-name").is_ok(), "new slug must exist");
}
