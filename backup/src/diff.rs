//! Binary diff and zstd compression for incremental backup payloads.

use anyhow::{Context, Result};
use std::io::{Cursor, Read};

const ZSTD_COMPRESSION_LEVEL: i32 = 3;

/// Hard ceiling on decompressed backup payload size.
///
/// Covers real SQLite databases comfortably while preventing crafted payloads
/// from exhausting memory via unbounded decompression.
pub const MAX_DECOMPRESSED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Computes a bsdiff binary patch from `old` to `new`.
pub fn compute_diff(old: &[u8], new: &[u8]) -> Result<Vec<u8>> {
    let mut patch = Vec::new();
    bsdiff::diff(old, new, &mut patch)?;
    Ok(patch)
}

/// Applies a bsdiff patch to `old`, producing the reconstructed `new` content.
pub fn apply_patch(old: &[u8], patch: &[u8]) -> Result<Vec<u8>> {
    let mut cursor = Cursor::new(patch);
    let mut output = Vec::new();
    bsdiff::patch(old, &mut cursor, &mut output)?;
    Ok(output)
}

/// Compresses data with zstd at the default backup compression level (3).
pub fn compress(data: &[u8]) -> Result<Vec<u8>> {
    let compressed = zstd::encode_all(data, ZSTD_COMPRESSION_LEVEL)?;
    Ok(compressed)
}

/// Decompresses zstd-compressed data back to its original form.
///
/// Rejects payloads that decompress beyond `MAX_DECOMPRESSED_BYTES` to prevent
/// zip-bomb attacks via crafted backup entries.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    decompress_with_cap(data, MAX_DECOMPRESSED_BYTES)
}

fn decompress_with_cap(data: &[u8], cap: u64) -> Result<Vec<u8>> {
    let mut decoder = zstd::stream::Decoder::new(data)
        .context("failed to initialize zstd decoder")?;
    let mut out = Vec::new();
    let read = (&mut decoder)
        .take(cap + 1)
        .read_to_end(&mut out)
        .context("failed to decompress backup payload")?;
    anyhow::ensure!(
        read as u64 <= cap,
        "decompressed backup payload exceeds {} byte cap",
        cap
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_and_patch_round_trip() {
        let old = b"hello world, this is the old content with some data";
        let new = b"hello world, this is the new content with some data";

        let patch = compute_diff(old, new).unwrap();
        let restored = apply_patch(old, &patch).unwrap();

        assert_eq!(restored, new);
    }

    #[test]
    fn diff_identical_inputs_produces_small_patch() {
        // bsdiff stores diff bytes as (new XOR old), so identical inputs produce
        // an all-zero diff stream that compresses extremely well. The compressed
        // patch should be far smaller than the raw new content.
        let data: Vec<u8> = (0u8..=255).cycle().take(10_000).collect();

        let patch = compute_diff(&data, &data).unwrap();
        let compressed_patch = compress(&patch).unwrap();

        assert!(compressed_patch.len() < data.len() / 2);
    }

    #[test]
    fn compress_decompress_round_trip() {
        let original = b"some data to compress and decompress";

        let compressed = compress(original).unwrap();
        let decompressed = decompress(&compressed).unwrap();

        assert_eq!(decompressed, original);
    }

    #[test]
    fn compress_reduces_size_for_repetitive_data() {
        let repetitive: Vec<u8> = b"abcdefgh".iter().cycle().take(10_000).copied().collect();

        let compressed = compress(&repetitive).unwrap();

        assert!(compressed.len() < repetitive.len());
    }

    #[test]
    fn decompress_rejects_payload_exceeding_cap() {
        let repetitive: Vec<u8> = b"aaaaaaaaaaaaaaaa".iter().cycle().take(1_024).copied().collect();
        let compressed = compress(&repetitive).unwrap();

        let result = decompress_with_cap(&compressed, 512);

        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("exceeds"), "unexpected error: {msg}");
    }
}
