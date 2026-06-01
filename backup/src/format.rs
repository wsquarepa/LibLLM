//! On-disk layout for self-describing encrypted base backup files.
//!
//! A type-3 base file embeds the chain's wrapped DEK ahead of the payload so the
//! data-encryption key travels with the ciphertext, letting the index be rebuilt
//! from disk alone:
//!
//! ```text
//! MAGIC(4) | format_version(1) | wrapped_dek_len(u16 BE) | wrapped_dek | payload
//! ```

use crate::index::WrappedDek;

const MAGIC: [u8; 4] = *b"LBKD";
const FORMAT_VERSION: u8 = 1;
const HEADER_PREFIX_LEN: usize = MAGIC.len() + 1 + 2;

/// Prepends the type-3 header carrying `wrapped` to `payload`.
pub fn encode_base_blob(wrapped: &WrappedDek, payload: &[u8]) -> Vec<u8> {
    let dek_len = wrapped.blob.len() as u16;
    let mut out = Vec::with_capacity(HEADER_PREFIX_LEN + wrapped.blob.len() + payload.len());
    out.extend_from_slice(&MAGIC);
    out.push(FORMAT_VERSION);
    out.extend_from_slice(&dek_len.to_be_bytes());
    out.extend_from_slice(&wrapped.blob);
    out.extend_from_slice(payload);
    out
}

/// Splits a type-3 base file into its wrapped DEK and payload slice.
///
/// Returns `None` when the magic is absent, the format version is unknown, or the
/// header is truncated. Callers treat `None` as a headerless (legacy) file and may
/// also fall back to `None` handling if a matched magic later fails to decrypt,
/// which keeps the ~2^-32 random-nonce collision non-fatal.
pub fn decode_base_blob(bytes: &[u8]) -> Option<(WrappedDek, &[u8])> {
    if bytes.len() < HEADER_PREFIX_LEN || bytes[..MAGIC.len()] != MAGIC {
        return None;
    }
    if bytes[MAGIC.len()] != FORMAT_VERSION {
        return None;
    }
    let dek_len = u16::from_be_bytes([bytes[5], bytes[6]]) as usize;
    let dek_end = HEADER_PREFIX_LEN.checked_add(dek_len)?;
    if bytes.len() < dek_end {
        return None;
    }
    let blob = bytes[HEADER_PREFIX_LEN..dek_end].to_vec();
    let payload = &bytes[dek_end..];
    Some((WrappedDek { blob }, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_then_decode_round_trips() {
        let wrapped = WrappedDek {
            blob: vec![7u8; 72],
        };
        let payload = b"ciphertext-payload-bytes";
        let encoded = encode_base_blob(&wrapped, payload);
        let (decoded_wrapped, decoded_payload) = decode_base_blob(&encoded).expect("decodes");
        assert_eq!(decoded_wrapped.blob, wrapped.blob);
        assert_eq!(decoded_payload, payload);
    }

    #[test]
    fn decode_returns_none_for_headerless_bytes() {
        let raw = vec![0xABu8; 96];
        assert!(decode_base_blob(&raw).is_none());
    }

    #[test]
    fn decode_returns_none_for_truncated_header() {
        let wrapped = WrappedDek {
            blob: vec![3u8; 72],
        };
        let encoded = encode_base_blob(&wrapped, b"payload");
        let truncated = &encoded[..HEADER_PREFIX_LEN + 10];
        assert!(decode_base_blob(truncated).is_none());
    }

    #[test]
    fn decode_returns_none_for_magic_only_too_short() {
        let mut bytes = MAGIC.to_vec();
        bytes.push(FORMAT_VERSION);
        assert!(decode_base_blob(&bytes).is_none());
    }
}
