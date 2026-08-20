// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
//! Versioned encoded-blob representation for generated static data tables.
//!
//! Generated recognizers historically embedded their lexer DFA tables, packed
//! parser ATN, and serialized lexer ATN as decimal Rust integer arrays. Large
//! grammars turned those arrays into hundreds of thousands of integer tokens
//! and multi-megabyte single source lines before rustc handed the runtime the
//! already-packed data. This module defines the compact source representation
//! the generator emits instead: one Rust string literal that this runtime
//! decodes back into the exact integer stream at first use.
//!
//! # Format
//!
//! The **text layer** is canonical unpadded base64 over the RFC 4648 standard
//! alphabet (`A`-`Z`, `a`-`z`, `0`-`9`, `+`, `/`). Padding characters and
//! whitespace are rejected, and the unused low bits of a trailing partial
//! group must be zero, so every byte payload has exactly one valid encoding.
//!
//! The **byte layer** is host-independent; it never serializes Rust struct
//! memory and has no endianness or alignment requirements:
//!
//! | Offset | Content |
//! | --- | --- |
//! | 0..4 | magic `b"AR4B"` |
//! | 4 | format version (currently 1) |
//! | 5 | element kind: 1 = `u32`, 2 = zigzag-mapped `i32` |
//! | 6.. | element count as one LEB128 varint |
//! | then | `count` LEB128 varints, one per element |
//!
//! Every integer is a **LEB128 varint**: little-endian base-128 groups of
//! seven value bits per byte, high bit set on every byte except the last, at
//! most five bytes for a 32-bit value. Encodings must be minimal; an overlong
//! form (a multi-byte varint whose final byte is zero) or a fifth byte
//! carrying bits above bit 31 is malformed. Kind 2 stores each `i32` through
//! the zigzag mapping `(value << 1) ^ (value >> 31)` so small negative values
//! stay short. Decoding validates the magic, version, kind, declared count,
//! every varint, and that no trailing bytes follow the last element, then
//! fails with a targeted [`EncodedBlobError`] naming the first violation.
//!
//! The payload carried inside a blob keeps its own validation: the compiled
//! lexer DFA stream is tag-guarded and the packed parser ATN carries its own
//! magic and format version, so this container only guards the source-level
//! transport.

/// Magic bytes opening every encoded blob.
const BLOB_MAGIC: [u8; 4] = *b"AR4B";

/// Current (and only) encoded-blob format version.
pub const ENCODED_BLOB_VERSION: u8 = 1;

/// Element kind byte for plain `u32` values.
const KIND_U32: u8 = 1;

/// Element kind byte for zigzag-mapped `i32` values.
const KIND_I32_ZIGZAG: u8 = 2;

/// Fixed byte length of the magic/version/kind prefix.
const PREFIX_LEN: usize = 6;

/// Failure while decoding an encoded blob back into its integer stream.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EncodedBlobError {
    /// The text layer contains a byte outside the unpadded base64 alphabet.
    #[error("invalid base64 byte 0x{byte:02x} at offset {offset}")]
    InvalidBase64Byte {
        /// Offending byte value.
        byte: u8,
        /// Byte offset within the encoded text.
        offset: usize,
    },
    /// The text length leaves a single trailing base64 character, which
    /// cannot encode a byte.
    #[error("truncated base64 text: one trailing character cannot encode a byte")]
    TruncatedBase64,
    /// The unused low bits of the final partial base64 group are not zero.
    #[error("non-canonical base64 text: unused trailing bits are not zero")]
    NonCanonicalBase64,
    /// The byte payload ends before the fixed magic/version/kind prefix.
    #[error("encoded blob holds {found} bytes, shorter than its {PREFIX_LEN}-byte header")]
    TruncatedHeader {
        /// Bytes actually present.
        found: usize,
    },
    /// The payload does not open with the `AR4B` magic.
    #[error("encoded blob magic mismatch: expected \"AR4B\", found {found:?}")]
    BadMagic {
        /// Bytes found where the magic was expected.
        found: [u8; 4],
    },
    /// The payload declares a format version this runtime does not read.
    #[error(
        "encoded blob format version {found} is unsupported; \
         this runtime reads version {ENCODED_BLOB_VERSION}"
    )]
    UnsupportedVersion {
        /// Version byte found in the payload.
        found: u8,
    },
    /// The payload carries a different element kind than the caller expects.
    #[error("encoded blob carries element kind {found}, but the caller expects kind {expected}")]
    ElementKindMismatch {
        /// Kind byte the decoding entry point expects.
        expected: u8,
        /// Kind byte found in the payload.
        found: u8,
    },
    /// The element-count varint is truncated, overlong, or exceeds 32 bits.
    #[error("encoded blob element count is truncated or malformed")]
    InvalidCount,
    /// The declared element count cannot fit in the remaining payload bytes.
    #[error(
        "encoded blob declares {declared} elements, but only {available_bytes} \
         payload bytes remain"
    )]
    TruncatedPayload {
        /// Elements the header declares.
        declared: usize,
        /// Payload bytes remaining after the count.
        available_bytes: usize,
    },
    /// A value varint runs past the end of the payload.
    #[error("encoded blob value at index {index} is truncated")]
    TruncatedValue {
        /// Zero-based index of the unfinished element.
        index: usize,
    },
    /// A value varint is overlong or exceeds 32 bits.
    #[error("encoded blob value at index {index} is malformed")]
    MalformedValue {
        /// Zero-based index of the malformed element.
        index: usize,
    },
    /// Payload bytes remain after the declared final element.
    #[error("encoded blob carries {count} trailing bytes after its declared elements")]
    TrailingBytes {
        /// Number of undeclared trailing bytes.
        count: usize,
    },
}

/// Encodes `u32` values into the deterministic blob text.
pub fn encode_u32_values(values: &[u32]) -> String {
    encode_values(KIND_U32, values.iter().copied(), values.len())
}

/// Encodes `i32` values into the deterministic blob text via zigzag mapping.
pub fn encode_i32_values(values: &[i32]) -> String {
    encode_values(
        KIND_I32_ZIGZAG,
        values.iter().copied().map(zigzag),
        values.len(),
    )
}

/// Decodes blob text produced by [`encode_u32_values`].
pub fn decode_u32_values(text: &str) -> Result<Vec<u32>, EncodedBlobError> {
    decode_values(text, KIND_U32)
}

/// Decodes blob text produced by [`encode_i32_values`].
pub fn decode_i32_values(text: &str) -> Result<Vec<i32>, EncodedBlobError> {
    let values = decode_values(text, KIND_I32_ZIGZAG)?;
    Ok(values.into_iter().map(unzigzag).collect())
}

/// Maps an `i32` onto the zigzag `u32` line so small magnitudes stay short.
const fn zigzag(value: i32) -> u32 {
    ((value << 1) ^ (value >> 31)).cast_unsigned()
}

/// Inverts [`zigzag`].
const fn unzigzag(value: u32) -> i32 {
    (value >> 1).cast_signed() ^ -((value & 1).cast_signed())
}

/// Serializes one header plus varint stream and armors it as base64 text.
fn encode_values(kind: u8, values: impl Iterator<Item = u32>, count: usize) -> String {
    let count_u32 = u32::try_from(count)
        .expect("encoded blobs carry at most u32::MAX elements by construction");
    // Most table words are small; two bytes per value is a fair pre-size.
    let mut bytes = Vec::with_capacity(PREFIX_LEN + 5 + count * 2);
    bytes.extend_from_slice(&BLOB_MAGIC);
    bytes.push(ENCODED_BLOB_VERSION);
    bytes.push(kind);
    write_varint(&mut bytes, count_u32);
    for value in values {
        write_varint(&mut bytes, value);
    }
    encode_base64(&bytes)
}

/// Appends one minimal LEB128 varint.
fn write_varint(out: &mut Vec<u8>, mut value: u32) {
    while value >= 0x80 {
        out.push((value & 0x7F) as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// Reads one minimal LEB128 varint, rejecting truncated, overlong, and
/// 32-bit-overflowing encodings.
fn read_varint(bytes: &[u8], position: &mut usize) -> Result<u32, VarintFailure> {
    let mut value: u32 = 0;
    for size in 0..5 {
        let Some(&byte) = bytes.get(*position) else {
            return Err(VarintFailure::Truncated);
        };
        *position += 1;
        let group = u32::from(byte & 0x7F);
        // The fifth byte may only carry bits 28..=31.
        if size == 4 && group > 0x0F {
            return Err(VarintFailure::Malformed);
        }
        value |= group << (size * 7);
        if byte < 0x80 {
            // A zero final byte in a multi-byte varint is an overlong form;
            // the encoder always emits the minimal encoding.
            if size > 0 && group == 0 {
                return Err(VarintFailure::Malformed);
            }
            return Ok(value);
        }
    }
    Err(VarintFailure::Malformed)
}

/// Distinguishes the two varint failure shapes for targeted diagnostics.
enum VarintFailure {
    Truncated,
    Malformed,
}

/// Validates the header and decodes the raw (pre-zigzag) value stream.
fn decode_values(text: &str, expected_kind: u8) -> Result<Vec<u32>, EncodedBlobError> {
    let bytes = decode_base64(text)?;
    if bytes.len() < PREFIX_LEN {
        return Err(EncodedBlobError::TruncatedHeader { found: bytes.len() });
    }
    let magic: [u8; 4] = bytes[..4]
        .try_into()
        .expect("a four-byte slice converts into a four-byte array");
    if magic != BLOB_MAGIC {
        return Err(EncodedBlobError::BadMagic { found: magic });
    }
    if bytes[4] != ENCODED_BLOB_VERSION {
        return Err(EncodedBlobError::UnsupportedVersion { found: bytes[4] });
    }
    if bytes[5] != expected_kind {
        return Err(EncodedBlobError::ElementKindMismatch {
            expected: expected_kind,
            found: bytes[5],
        });
    }
    let mut position = PREFIX_LEN;
    let count =
        read_varint(&bytes, &mut position).map_err(|_| EncodedBlobError::InvalidCount)? as usize;
    let available_bytes = bytes.len() - position;
    // Every element occupies at least one byte, so the declared count bounds
    // the remaining payload before any allocation happens.
    if count > available_bytes {
        return Err(EncodedBlobError::TruncatedPayload {
            declared: count,
            available_bytes,
        });
    }
    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        let value = read_varint(&bytes, &mut position).map_err(|failure| match failure {
            VarintFailure::Truncated => EncodedBlobError::TruncatedValue { index },
            VarintFailure::Malformed => EncodedBlobError::MalformedValue { index },
        })?;
        values.push(value);
    }
    if position != bytes.len() {
        return Err(EncodedBlobError::TrailingBytes {
            count: bytes.len() - position,
        });
    }
    Ok(values)
}

/// RFC 4648 standard base64 alphabet.
const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Sextet marker for bytes outside the base64 alphabet.
const BASE64_INVALID: u8 = 0xFF;

/// Byte-to-sextet reverse table; [`BASE64_INVALID`] marks foreign bytes.
static BASE64_SEXTETS: [u8; 256] = {
    let mut table = [BASE64_INVALID; 256];
    let mut index = 0;
    while index < BASE64_ALPHABET.len() {
        table[BASE64_ALPHABET[index] as usize] = index as u8;
        index += 1;
    }
    table
};

/// Armors bytes as canonical unpadded base64 text.
fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut chunks = bytes.chunks_exact(3);
    for chunk in chunks.by_ref() {
        let group = u32::from(chunk[0]) << 16 | u32::from(chunk[1]) << 8 | u32::from(chunk[2]);
        out.push(BASE64_ALPHABET[(group >> 18) as usize & 0x3F] as char);
        out.push(BASE64_ALPHABET[(group >> 12) as usize & 0x3F] as char);
        out.push(BASE64_ALPHABET[(group >> 6) as usize & 0x3F] as char);
        out.push(BASE64_ALPHABET[group as usize & 0x3F] as char);
    }
    match *chunks.remainder() {
        [first] => {
            let group = u32::from(first) << 16;
            out.push(BASE64_ALPHABET[(group >> 18) as usize & 0x3F] as char);
            out.push(BASE64_ALPHABET[(group >> 12) as usize & 0x3F] as char);
        }
        [first, second] => {
            let group = u32::from(first) << 16 | u32::from(second) << 8;
            out.push(BASE64_ALPHABET[(group >> 18) as usize & 0x3F] as char);
            out.push(BASE64_ALPHABET[(group >> 12) as usize & 0x3F] as char);
            out.push(BASE64_ALPHABET[(group >> 6) as usize & 0x3F] as char);
        }
        _ => {}
    }
    out
}

/// Decodes canonical unpadded base64 text, rejecting foreign bytes, padding,
/// impossible lengths, and non-zero trailing bits.
fn decode_base64(text: &str) -> Result<Vec<u8>, EncodedBlobError> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 + 2);
    let mut chunks = bytes.chunks_exact(4);
    for (chunk_index, chunk) in chunks.by_ref().enumerate() {
        let first = BASE64_SEXTETS[chunk[0] as usize];
        let second = BASE64_SEXTETS[chunk[1] as usize];
        let third = BASE64_SEXTETS[chunk[2] as usize];
        let fourth = BASE64_SEXTETS[chunk[3] as usize];
        if (first | second | third | fourth) == BASE64_INVALID {
            return Err(invalid_base64_byte(text, chunk_index * 4));
        }
        let group = u32::from(first) << 18
            | u32::from(second) << 12
            | u32::from(third) << 6
            | u32::from(fourth);
        out.extend_from_slice(&[(group >> 16) as u8, (group >> 8) as u8, group as u8]);
    }
    let tail_offset = bytes.len() - chunks.remainder().len();
    match *chunks.remainder() {
        [] => {}
        [_] => {
            if BASE64_SEXTETS[chunks.remainder()[0] as usize] == BASE64_INVALID {
                return Err(invalid_base64_byte(text, tail_offset));
            }
            return Err(EncodedBlobError::TruncatedBase64);
        }
        [first, second] => {
            let first = BASE64_SEXTETS[first as usize];
            let second = BASE64_SEXTETS[second as usize];
            if (first | second) == BASE64_INVALID {
                return Err(invalid_base64_byte(text, tail_offset));
            }
            if second & 0x0F != 0 {
                return Err(EncodedBlobError::NonCanonicalBase64);
            }
            out.push(first << 2 | second >> 4);
        }
        [first, second, third] => {
            let first = BASE64_SEXTETS[first as usize];
            let second = BASE64_SEXTETS[second as usize];
            let third = BASE64_SEXTETS[third as usize];
            if (first | second | third) == BASE64_INVALID {
                return Err(invalid_base64_byte(text, tail_offset));
            }
            if third & 0x03 != 0 {
                return Err(EncodedBlobError::NonCanonicalBase64);
            }
            out.push(first << 2 | second >> 4);
            out.push(second << 4 | third >> 2);
        }
        _ => unreachable!("chunks_exact(4) leaves at most three remainder bytes"),
    }
    Ok(out)
}

/// Locates the first foreign byte at or after `from` for a targeted error.
fn invalid_base64_byte(text: &str, from: usize) -> EncodedBlobError {
    text.bytes()
        .enumerate()
        .skip(from)
        .find(|&(_, byte)| BASE64_SEXTETS[byte as usize] == BASE64_INVALID)
        .map_or(
            // A validation failure always has an offending byte; keep a
            // defensive fallback instead of panicking inside error reporting.
            EncodedBlobError::TruncatedBase64,
            |(offset, byte)| EncodedBlobError::InvalidBase64Byte { byte, offset },
        )
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
mod tests {
    use super::*;

    #[test]
    fn u32_round_trip_covers_varint_width_boundaries() {
        let values = [
            0_u32,
            1,
            0x7F,
            0x80,
            0x3FFF,
            0x4000,
            0x001F_FFFF,
            0x0020_0000,
            0x0FFF_FFFF,
            0x1000_0000,
            u32::MAX,
        ];
        let encoded = encode_u32_values(&values);
        assert_eq!(
            decode_u32_values(&encoded).expect("boundary values should round-trip"),
            values
        );
    }

    #[test]
    fn i32_round_trip_covers_sign_boundaries() {
        let values = [0_i32, 1, -1, 63, -64, 64, -65, i32::MAX, i32::MIN];
        let encoded = encode_i32_values(&values);
        assert_eq!(
            decode_i32_values(&encoded).expect("signed boundary values should round-trip"),
            values
        );
    }

    #[test]
    fn empty_streams_round_trip() {
        assert_eq!(
            decode_u32_values(&encode_u32_values(&[])).expect("empty u32 stream should decode"),
            Vec::<u32>::new()
        );
        assert_eq!(
            decode_i32_values(&encode_i32_values(&[])).expect("empty i32 stream should decode"),
            Vec::<i32>::new()
        );
    }

    #[test]
    fn encoding_is_deterministic_and_ascii() {
        let values = [7_u32, 0, u32::MAX, 300];
        let first = encode_u32_values(&values);
        assert_eq!(first, encode_u32_values(&values));
        assert!(first.is_ascii());
        insta::assert_snapshot!("encoded_blob_small_u32_stream", first);
    }

    #[test]
    fn decode_diagnostics_name_the_first_violation() {
        let valid = encode_u32_values(&[1, 2, 3]);

        let mut cases: Vec<(&str, String)> = Vec::new();
        let invalid_byte = decode_u32_values("QVI0!QgE").expect_err("bad byte should fail");
        cases.push(("invalid byte", invalid_byte.to_string()));
        let padded = decode_u32_values(&format!("{valid}=")).expect_err("padding should fail");
        cases.push(("rejected padding", padded.to_string()));
        let truncated_base64 = decode_u32_values("Q").expect_err("length % 4 == 1 should fail");
        cases.push(("truncated base64", truncated_base64.to_string()));
        let non_canonical = decode_u32_values("QR").expect_err("dirty trailing bits should fail");
        cases.push(("non-canonical base64", non_canonical.to_string()));
        let truncated_header =
            decode_u32_values(&encode_base64(b"AR4")).expect_err("short payload should fail");
        cases.push(("truncated header", truncated_header.to_string()));
        let bad_magic = decode_u32_values(&encode_base64(b"NOPE\x01\x01\x00"))
            .expect_err("foreign magic should fail");
        cases.push(("bad magic", bad_magic.to_string()));
        let unsupported_version = decode_u32_values(&encode_base64(b"AR4B\x02\x01\x00"))
            .expect_err("future version should fail");
        cases.push(("unsupported version", unsupported_version.to_string()));
        let kind_mismatch =
            decode_u32_values(&encode_i32_values(&[1])).expect_err("i32 blob should be rejected");
        cases.push(("element kind mismatch", kind_mismatch.to_string()));
        let invalid_count = decode_u32_values(&encode_base64(b"AR4B\x01\x01\xFF"))
            .expect_err("truncated count varint should fail");
        cases.push(("invalid count", invalid_count.to_string()));
        let truncated_payload = decode_u32_values(&encode_base64(b"AR4B\x01\x01\x03\x01"))
            .expect_err("count larger than payload should fail");
        cases.push(("truncated payload", truncated_payload.to_string()));
        let truncated_value = decode_u32_values(&encode_base64(b"AR4B\x01\x01\x02\x01\xFF"))
            .expect_err("unfinished value varint should fail");
        cases.push(("truncated value", truncated_value.to_string()));
        let overflowing_value =
            decode_u32_values(&encode_base64(b"AR4B\x01\x01\x01\xFF\xFF\xFF\xFF\x7F"))
                .expect_err("33-bit value should fail");
        cases.push(("overflowing value", overflowing_value.to_string()));
        let overlong_value = decode_u32_values(&encode_base64(b"AR4B\x01\x01\x02\x80\x00\x01"))
            .expect_err("overlong varint should fail");
        cases.push(("overlong value", overlong_value.to_string()));
        let trailing_bytes = decode_u32_values(&encode_base64(b"AR4B\x01\x01\x01\x01\x00"))
            .expect_err("undeclared trailing byte should fail");
        cases.push(("trailing bytes", trailing_bytes.to_string()));

        let report = cases
            .iter()
            .map(|(name, message)| format!("{name}: {message}"))
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!("encoded_blob_decode_diagnostics", report);
    }

    #[test]
    fn overlong_count_varint_is_rejected() {
        // Count `1` encoded as the overlong two-byte form 0x81 0x00.
        let err = decode_u32_values(&encode_base64(b"AR4B\x01\x01\x81\x00\x01"))
            .expect_err("overlong count varint should fail");
        assert_eq!(err, EncodedBlobError::InvalidCount);
    }

    #[test]
    fn zigzag_mapping_matches_reference_pairs() {
        // Reference pairs from the protobuf zigzag definition.
        let pairs = [
            (0_i32, 0_u32),
            (-1, 1),
            (1, 2),
            (-2, 3),
            (i32::MAX, u32::MAX - 1),
            (i32::MIN, u32::MAX),
        ];
        for (signed, unsigned) in pairs {
            assert_eq!(zigzag(signed), unsigned, "zigzag({signed})");
            assert_eq!(unzigzag(unsigned), signed, "unzigzag({unsigned})");
        }
    }
}
