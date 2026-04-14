use anyhow::Result;
use std::io::Cursor;

const ZSTD_COMPRESSION_LEVEL: i32 = 3;

pub fn compute_diff(old: &[u8], new: &[u8]) -> Result<Vec<u8>> {
    let mut patch = Vec::new();
    bsdiff::diff(old, new, &mut patch)?;
    Ok(patch)
}

pub fn apply_patch(old: &[u8], patch: &[u8]) -> Result<Vec<u8>> {
    let mut cursor = Cursor::new(patch);
    let mut output = Vec::new();
    bsdiff::patch(old, &mut cursor, &mut output)?;
    Ok(output)
}

pub fn compress(data: &[u8]) -> Result<Vec<u8>> {
    let compressed = zstd::encode_all(data, ZSTD_COMPRESSION_LEVEL)?;
    Ok(compressed)
}

pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    let decompressed = zstd::decode_all(data)?;
    Ok(decompressed)
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
}
