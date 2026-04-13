// Each test binary only uses a subset of shared helpers; allow unused ones.
#[allow(dead_code)]
mod common;

use libllm_core::crypto;
use libllm_core::session::{self, MessageTree, Role, Session};

// ===========================================================================
// 1. Session & MessageTree
// ===========================================================================

#[test]
fn session_plaintext_round_trip() {
    let dir = common::temp_dir();
    common::create_data_dirs(dir.path());
    let path = common::session_path(dir.path(), "plain.session");

    let session = common::linear_session(vec![
        common::user_msg("hello"),
        common::assistant_msg("hi there"),
        common::user_msg("how are you?"),
    ]);
    session::save(&path, &session).expect("save failed");
    let loaded = session::load(&path).expect("load failed");

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
fn session_encrypted_round_trip() {
    let dir = common::temp_dir();
    common::create_data_dirs(dir.path());
    let key = common::test_key(dir.path());
    let path = common::session_path(dir.path(), "enc.session");

    let session = common::linear_session(vec![
        common::user_msg("secret question"),
        common::assistant_msg("secret answer"),
    ]);
    session::save_encrypted(&path, &session, &key).expect("save_encrypted failed");
    let loaded = session::load_encrypted(&path, &key).expect("load_encrypted failed");

    let messages = loaded.tree.branch_path();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "secret question");
    assert_eq!(messages[1].content, "secret answer");
}

#[test]
fn session_wrong_key_rejected() {
    let dir = common::temp_dir();
    common::create_data_dirs(dir.path());
    let key_a = common::test_key(dir.path());

    let salt_b_path = dir.path().join(".salt_b");
    let salt_b = crypto::load_or_create_salt(&salt_b_path).expect("salt_b");
    let key_b = crypto::derive_key("different-passkey", &salt_b).expect("derive key_b");

    let path = common::session_path(dir.path(), "wrong_key.session");
    let session = common::linear_session(vec![common::user_msg("private")]);
    session::save_encrypted(&path, &session, &key_a).expect("save failed");

    let result = session::load_encrypted(&path, &key_b);
    assert!(result.is_err(), "loading with wrong key should fail");
}

#[test]
fn session_branching() {
    let mut tree = MessageTree::new();

    let root = tree.push(None, common::user_msg("root"));
    let a1 = tree.push(Some(root), common::assistant_msg("branch A"));
    let _a2 = tree.push(Some(a1), common::user_msg("continue A"));

    // Fork from root to create branch B
    let b1 = tree.push(Some(root), common::assistant_msg("branch B"));

    // Head is now b1; branch_path should be [root, b1]
    let path = tree.branch_path();
    assert_eq!(path.len(), 2);
    assert_eq!(path[0].content, "root");
    assert_eq!(path[1].content, "branch B");

    // Root node should have two children (a1 and b1)
    let root_node = tree.node(root).expect("root node");
    assert_eq!(root_node.children.len(), 2);

    // Siblings of b1 should include a1
    let siblings = tree.siblings_of(b1);
    assert!(siblings.contains(&a1));
    assert!(siblings.contains(&b1));
}

#[test]
fn session_empty_round_trip() {
    let dir = common::temp_dir();
    common::create_data_dirs(dir.path());
    let path = common::session_path(dir.path(), "empty.session");

    let session = common::linear_session(vec![]);
    session::save(&path, &session).expect("save empty");
    let loaded = session::load(&path).expect("load empty");

    assert_eq!(loaded.tree.branch_path().len(), 0);
    assert!(loaded.tree.head().is_none());
}

#[test]
fn session_metadata_fields_survive_round_trip() {
    let dir = common::temp_dir();
    common::create_data_dirs(dir.path());
    let path = common::session_path(dir.path(), "meta.session");

    let session = Session {
        tree: MessageTree::new(),
        model: Some("llama-3".to_string()),
        template: Some("chatml".to_string()),
        system_prompt: Some("Be helpful.".to_string()),
        character: Some("TestChar".to_string()),
        worldbooks: vec!["lore-a".to_string(), "lore-b".to_string()],
        persona: Some("Alice".to_string()),
    };
    session::save(&path, &session).expect("save meta");
    let loaded = session::load(&path).expect("load meta");

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

    // Duplicate the subtree rooted at n1
    let copy_root = tree.duplicate_subtree(n1).expect("duplicate_subtree");

    // The copy should be a different node
    assert_ne!(copy_root, n1);

    // Modifying the copy should not affect the original
    tree.set_message_content(copy_root, "modified copy".to_string());
    let original = tree.node(n1).expect("original node");
    assert_eq!(original.message.content, "second");

    let copy = tree.node(copy_root).expect("copy node");
    assert_eq!(copy.message.content, "modified copy");
}

#[test]
fn session_set_message_content() {
    let dir = common::temp_dir();
    common::create_data_dirs(dir.path());
    let path = common::session_path(dir.path(), "edit.session");

    let mut session = common::linear_session(vec![
        common::user_msg("original"),
        common::assistant_msg("reply"),
    ]);

    let head_id = session.tree.head().expect("has head");
    let updated = session
        .tree
        .set_message_content(head_id, "edited reply".to_string());
    assert!(updated, "set_message_content should return true");

    session::save(&path, &session).expect("save");
    let loaded = session::load(&path).expect("load");

    let messages = loaded.tree.branch_path();
    assert_eq!(messages[1].content, "edited reply");
}

// ===========================================================================
// 2. Crypto
// ===========================================================================

#[test]
fn crypto_encrypt_decrypt_round_trip() {
    let dir = common::temp_dir();
    let key = common::test_key(dir.path());
    let plaintext = b"the quick brown fox jumps over the lazy dog";

    let blob = crypto::encrypt(plaintext, &key).expect("encrypt");
    let decrypted = crypto::decrypt(&blob, &key).expect("decrypt");
    assert_eq!(decrypted, plaintext);
}

#[test]
fn crypto_is_encrypted_detection() {
    let dir = common::temp_dir();
    let key = common::test_key(dir.path());

    let blob = crypto::encrypt(b"data", &key).expect("encrypt");
    assert!(crypto::is_encrypted(&blob));

    assert!(!crypto::is_encrypted(b"just plain text"));
    assert!(!crypto::is_encrypted(b"LLM")); // too short for magic
    assert!(!crypto::is_encrypted(b""));
}

#[test]
fn crypto_salt_persistence() {
    let dir = common::temp_dir();
    let salt_path = dir.path().join(".salt");

    let salt1 = crypto::load_or_create_salt(&salt_path).expect("first call");
    let salt2 = crypto::load_or_create_salt(&salt_path).expect("second call");
    assert_eq!(salt1, salt2);
}

#[test]
fn crypto_key_determinism() {
    let dir = common::temp_dir();
    let salt_path = dir.path().join(".salt");
    let salt = crypto::load_or_create_salt(&salt_path).expect("salt");

    let key1 = crypto::derive_key("same-passkey", &salt).expect("key1");
    let key2 = crypto::derive_key("same-passkey", &salt).expect("key2");

    // Verify determinism: encrypt with key1, decrypt with key2
    let blob = crypto::encrypt(b"test", &key1).expect("encrypt");
    let decrypted = crypto::decrypt(&blob, &key2).expect("decrypt with key2");
    assert_eq!(decrypted, b"test");
}

#[test]
fn crypto_different_passkeys_differ() {
    let dir = common::temp_dir();
    let salt_path = dir.path().join(".salt");
    let salt = crypto::load_or_create_salt(&salt_path).expect("salt");

    let key_a = crypto::derive_key("passkey-a", &salt).expect("key_a");
    let key_b = crypto::derive_key("passkey-b", &salt).expect("key_b");

    let blob = crypto::encrypt(b"secret", &key_a).expect("encrypt");
    let result = crypto::decrypt(&blob, &key_b);
    assert!(
        result.is_err(),
        "decrypting with a different key should fail"
    );
}

#[test]
fn crypto_encrypt_and_write_read_and_decrypt() {
    let dir = common::temp_dir();
    let key = common::test_key(dir.path());

    // Encrypted file round-trip
    let enc_path = dir.path().join("encrypted.bin");
    crypto::encrypt_and_write(&enc_path, b"encrypted content", Some(&key))
        .expect("encrypt_and_write");
    let decrypted = crypto::read_and_decrypt(&enc_path, Some(&key)).expect("read_and_decrypt");
    assert_eq!(decrypted, "encrypted content");

    // Plaintext file round-trip (key = None)
    let plain_path = dir.path().join("plain.bin");
    crypto::encrypt_and_write(&plain_path, b"plain content", None)
        .expect("encrypt_and_write plaintext");
    let plain_read =
        crypto::read_and_decrypt(&plain_path, None).expect("read_and_decrypt plaintext");
    assert_eq!(plain_read, "plain content");
}

#[test]
fn crypto_tampered_ciphertext_fails() {
    let dir = common::temp_dir();
    let key = common::test_key(dir.path());

    let mut blob = crypto::encrypt(b"important data", &key).expect("encrypt");

    // Tamper with a byte in the ciphertext region (after magic + version + nonce = 17 bytes)
    let tamper_idx = blob.len() - 1;
    blob[tamper_idx] ^= 0xFF;

    let result = crypto::decrypt(&blob, &key);
    assert!(
        result.is_err(),
        "tampered ciphertext should fail decryption"
    );
}

#[test]
fn crypto_empty_plaintext() {
    let dir = common::temp_dir();
    let key = common::test_key(dir.path());

    let blob = crypto::encrypt(b"", &key).expect("encrypt empty");
    let decrypted = crypto::decrypt(&blob, &key).expect("decrypt empty");
    assert!(decrypted.is_empty());
}
