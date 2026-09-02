#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failed setup step should panic at its call site"
)]

use std::path::Path;

/// Opens `db_path` as an SQLCipher database keyed by `passkey`, loading or creating the
/// salt at `salt_path` and deriving the key with the core crypto module.
pub fn open_encrypted_conn(
    db_path: &Path,
    salt_path: &Path,
    passkey: &str,
) -> rusqlite::Connection {
    let salt = libllm_core::crypto::load_or_create_salt(salt_path).unwrap();
    let key = libllm_core::crypto::derive_key(passkey, &salt).unwrap();
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute_batch(&key.key_pragma()).unwrap();
    conn
}
