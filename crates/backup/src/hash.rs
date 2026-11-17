//! BLAKE3 hashing utilities for backup integrity verification.

use std::path::Path;

use crate::error::{BackupError, Result};

/// Returns the BLAKE3 hash of `data` as a 64-character lowercase hex string.
pub fn hash_bytes(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

/// Streams a file through BLAKE3 and returns its hash as a hex string.
///
/// Copies the file through the hasher with `std::io::copy` so large files
/// (multi-hundred-MB backups) are not materialized into a heap buffer.
pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).map_err(|source| BackupError::HashOpenFile {
        path: path.to_owned(),
        source,
    })?;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher).map_err(|source| BackupError::HashReadFile {
        path: path.to_owned(),
        source,
    })?;
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn hash_bytes_is_blake3_lowercase_hex() {
        assert_eq!(
            hash_bytes(b"hello world"),
            "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24"
        );
    }

    #[test]
    fn hash_file_matches_hash_bytes() {
        let data = b"test file contents";
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(data).unwrap();
        tmp.flush().unwrap();

        let file_hash = hash_file(tmp.path()).unwrap();
        let bytes_hash = hash_bytes(data);
        assert_eq!(file_hash, bytes_hash);
    }
}
