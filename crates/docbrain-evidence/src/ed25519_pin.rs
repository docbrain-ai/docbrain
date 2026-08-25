// SPDX-License-Identifier: MIT
//! Pinned ed25519 verification (spec law 4, R1): the rule set is exactly
//! `ed25519-dalek=2.2.0/verify_strict`. `verify_pinned` is the ONLY
//! signature-verification entry point in this crate — a later grep-audit
//! enforces that no other code path calls `verify`/`verify_strict` directly.
//! The frozen cross-language vector suite (`vectors/ed25519_pin.json`,
//! loaded by `tests/vectors.rs`) is the arbiter of this pin's behavior.

use ed25519_dalek::{Signature, VerifyingKey};

/// The pinned rule set, named so both this crate and its docs/spec can quote
/// one exact string. Two implementations of the spec MUST agree bit-for-bit
/// with this library+function pair.
pub const PIN: &str = "ed25519-dalek=2.2.0/verify_strict";

/// The ONLY signature-verification entry point in this crate.
pub fn verify_pinned(vk: &VerifyingKey, msg: &[u8], sig: &Signature) -> bool {
    vk.verify_strict(msg, sig).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    #[test]
    fn verify_pinned_accepts_a_genuine_signature() {
        let sk = key(1);
        let msg = b"pinned verification round trip";
        let sig = sk.sign(msg);
        assert!(verify_pinned(&sk.verifying_key(), msg, &sig));
    }

    #[test]
    fn verify_pinned_rejects_a_tampered_message() {
        let sk = key(1);
        let sig = sk.sign(b"original message");
        assert!(!verify_pinned(&sk.verifying_key(), b"tampered message", &sig));
    }

    #[test]
    fn verify_pinned_rejects_the_wrong_key() {
        let sk = key(1);
        let other = key(2);
        let msg = b"signed by key 1";
        let sig = sk.sign(msg);
        assert!(!verify_pinned(&other.verifying_key(), msg, &sig));
    }
}
