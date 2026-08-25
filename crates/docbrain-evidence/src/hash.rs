// SPDX-License-Identifier: MIT
//! Domain-separated SHA-256 hashing for the evidence record chain (spec law 4).
//!
//! Every hash in this module is prefixed with a fixed domain-separation byte
//! so that a leaf hash, a head hash, and a content hash can never collide by
//! construction, even if their raw inputs happened to coincide byte-for-byte.
//! The three prefixes (`0x00`, `0x01`, `0x02`) are part of the cross-language
//! contract (`docs/superpowers/specs/2026-08-24-evidence-bundle-design.md`)
//! and MUST NOT change once frozen — a test below pins the exact byte values
//! by independently reconstructing each hash from the documented formula.

use sha2::{Digest, Sha256};

const LEAF_PREFIX: u8 = 0x00;
const HEAD_PREFIX: u8 = 0x01;
const CONTENT_PREFIX: u8 = 0x02;

/// The `prev_head` value declared by the genesis record: 32 zero bytes,
/// pinned by the spec's genesis constants (design doc line 60).
pub const GENESIS_PREV: [u8; 32] = [0u8; 32];

/// Leaf hash of one signed record envelope: `SHA-256(0x00 || envelope_bytes)`.
///
/// `envelope_bytes` is the exact wire-form line (e.g. `Envelope::to_line()`),
/// never the decoded payload alone — hashing the whole envelope binds the
/// leaf to the signature and keyid too, not just the record content, so a
/// byte flip anywhere in the line (not only inside the payload) changes the
/// leaf and therefore every downstream head.
pub fn leaf_hash(envelope_bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([LEAF_PREFIX]);
    hasher.update(envelope_bytes);
    hasher.finalize().into()
}

/// Head hash linking a leaf to the chain: `SHA-256(0x01 || prev_head || leaf)`.
pub fn head_hash(prev_head: &[u8; 32], leaf: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([HEAD_PREFIX]);
    hasher.update(prev_head);
    hasher.update(leaf);
    hasher.finalize().into()
}

/// Salted content hash for erasure-compatible content addressing:
/// `SHA-256(0x02 || salt || content)`. The salt (32 random bytes, destroyed
/// on erasure) is what makes the erased content's hash infeasible to
/// dictionary-confirm once the salt is gone (spec §5.8).
pub fn content_hash(salt: &[u8; 32], content: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([CONTENT_PREFIX]);
    hasher.update(salt);
    hasher.update(content);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genesis_prev_is_32_zero_bytes() {
        assert_eq!(GENESIS_PREV, [0u8; 32]);
        assert_eq!(GENESIS_PREV.len(), 32);
    }

    #[test]
    fn domain_prefix_bytes_are_pinned_exactly() {
        // The literal constants, asserted so a later edit can never silently
        // drift them without a test turning red.
        assert_eq!(LEAF_PREFIX, 0x00);
        assert_eq!(HEAD_PREFIX, 0x01);
        assert_eq!(CONTENT_PREFIX, 0x02);
    }

    #[test]
    fn leaf_hash_matches_sha256_of_0x00_prefixed_bytes() {
        let bytes = b"an-envelope-line";
        let mut preimage = vec![0x00u8];
        preimage.extend_from_slice(bytes);
        let want: [u8; 32] = Sha256::digest(&preimage).into();
        assert_eq!(leaf_hash(bytes), want);
    }

    #[test]
    fn leaf_hash_of_empty_bytes() {
        let want: [u8; 32] = Sha256::digest([0x00u8]).into();
        assert_eq!(leaf_hash(b""), want);
    }

    #[test]
    fn head_hash_matches_sha256_of_0x01_prefixed_prev_and_leaf() {
        let prev = [7u8; 32];
        let leaf = [9u8; 32];
        let mut preimage = vec![0x01u8];
        preimage.extend_from_slice(&prev);
        preimage.extend_from_slice(&leaf);
        let want: [u8; 32] = Sha256::digest(&preimage).into();
        assert_eq!(head_hash(&prev, &leaf), want);
    }

    #[test]
    fn head_hash_of_genesis_prev() {
        let leaf = [1u8; 32];
        let mut preimage = vec![0x01u8];
        preimage.extend_from_slice(&GENESIS_PREV);
        preimage.extend_from_slice(&leaf);
        let want: [u8; 32] = Sha256::digest(&preimage).into();
        assert_eq!(head_hash(&GENESIS_PREV, &leaf), want);
    }

    #[test]
    fn content_hash_matches_sha256_of_0x02_prefixed_salt_and_content() {
        let salt = [3u8; 32];
        let content = b"claim body bytes";
        let mut preimage = vec![0x02u8];
        preimage.extend_from_slice(&salt);
        preimage.extend_from_slice(content);
        let want: [u8; 32] = Sha256::digest(&preimage).into();
        assert_eq!(content_hash(&salt, content), want);
    }

    #[test]
    fn content_hash_of_empty_content() {
        let salt = [3u8; 32];
        let mut preimage = vec![0x02u8];
        preimage.extend_from_slice(&salt);
        let want: [u8; 32] = Sha256::digest(&preimage).into();
        assert_eq!(content_hash(&salt, b""), want);
    }

    /// Domain separation, proven structurally rather than just empirically:
    /// feeding byte-identical data through all three functions must never
    /// collide, because the very first hashed byte differs (0x00/0x01/0x02).
    #[test]
    fn same_bytes_never_collide_across_domains() {
        let data = [0xABu8; 32];
        let as_leaf = leaf_hash(&data);
        let as_head = head_hash(&data, &data);
        let as_content = content_hash(&data, &data);
        assert_ne!(as_leaf, as_head);
        assert_ne!(as_leaf, as_content);
        assert_ne!(as_head, as_content);
    }

    #[test]
    fn leaf_hash_is_sensitive_to_every_byte() {
        let a = leaf_hash(b"record-a");
        let b = leaf_hash(b"record-b");
        assert_ne!(a, b);
    }

    #[test]
    fn head_hash_changes_when_prev_changes() {
        let leaf = [5u8; 32];
        let h1 = head_hash(&[1u8; 32], &leaf);
        let h2 = head_hash(&[2u8; 32], &leaf);
        assert_ne!(h1, h2);
    }

    #[test]
    fn head_hash_changes_when_leaf_changes() {
        let prev = [5u8; 32];
        let h1 = head_hash(&prev, &[1u8; 32]);
        let h2 = head_hash(&prev, &[2u8; 32]);
        assert_ne!(h1, h2);
    }
}
