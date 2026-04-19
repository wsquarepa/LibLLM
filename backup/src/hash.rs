//! BLAKE3 hashing utilities for backup integrity verification.

use std::path::Path;

use anyhow::{Context, Result};

/// Returns the BLAKE3 hash of `data` as a 64-character lowercase hex string.
pub fn hash_bytes(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

/// Streams a file through BLAKE3 and returns its hash as a hex string.
///
/// Copies the file through the hasher with `std::io::copy` so large files
/// (multi-hundred-MB backups) are not materialized into a heap buffer.
pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("failed to open file: {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher)
        .with_context(|| format!("failed to read file: {}", path.display()))?;
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn hash_bytes_deterministic() {
        let input = b"hello world";
        let first = hash_bytes(input);
        let second = hash_bytes(input);
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_bytes_differs_for_different_input() {
        let a = hash_bytes(b"foo");
        let b = hash_bytes(b"bar");
        assert_ne!(a, b);
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
