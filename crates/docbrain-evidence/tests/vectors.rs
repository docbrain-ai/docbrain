// SPDX-License-Identifier: MIT
//! Loads the frozen `vectors/ed25519_pin.json` (T-G, the cross-language
//! ed25519 contract shared with the Python reference verifier — Task 14)
//! and asserts `verify_pinned` returns exactly the recorded boolean for
//! every vector. This test NEVER regenerates the file — that only happens
//! via `cargo test -p docbrain-evidence --features gen-vectors -- generate_vectors`,
//! deliberately, when the pin itself changes.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use docbrain_evidence::verify_pinned;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;

#[derive(Deserialize)]
struct VectorFile {
    pin: String,
    vectors: Vec<Vector>,
}

#[derive(Deserialize)]
struct Vector {
    name: String,
    vk_b64: String,
    msg_b64: String,
    sig_b64: String,
    expected: bool,
}

fn load() -> VectorFile {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/vectors/ed25519_pin.json");
    let bytes = std::fs::read(path).expect("vectors/ed25519_pin.json must be committed");
    serde_json::from_slice(&bytes).expect("vectors/ed25519_pin.json must be valid JSON")
}

#[test]
fn pin_matches_the_crate_constant() {
    let file = load();
    assert_eq!(file.pin, docbrain_evidence::PIN);
}

#[test]
fn suite_has_at_least_eight_valid_and_nine_adversarial_vectors() {
    let file = load();
    let valid = file.vectors.iter().filter(|v| v.expected).count();
    let adversarial = file.vectors.iter().filter(|v| !v.expected).count();
    assert!(valid >= 8, "expected >= 8 valid vectors, got {valid}");
    assert!(
        adversarial >= 9,
        "expected >= 9 adversarial vectors, got {adversarial}"
    );
}

#[test]
fn every_vector_matches_verify_pinned() {
    let file = load();
    assert!(!file.vectors.is_empty(), "vector suite must not be empty");

    let mut checked = 0usize;
    for v in &file.vectors {
        let vk_bytes = STANDARD.decode(&v.vk_b64).expect("vk_b64 must decode");
        let msg = STANDARD.decode(&v.msg_b64).expect("msg_b64 must decode");
        let sig_bytes = STANDARD.decode(&v.sig_b64).expect("sig_b64 must decode");

        // A signature must be exactly 64 bytes to even be constructed; a
        // vector whose sig_b64 decodes to any other length (e.g. the
        // "truncated sig" adversarial case) is rejected here, at the same
        // gate a real envelope parser applies before it can hand anything
        // to verify_pinned. This is the honest way to represent "truncated
        // signature" as a vector: verify_pinned's own signature requires an
        // already-64-byte Signature, so the length check that rejects a
        // truncated one necessarily lives in the caller, not inside
        // verify_pinned itself.
        let got = if sig_bytes.len() != 64 {
            false
        } else {
            let vk_arr: [u8; 32] = vk_bytes
                .as_slice()
                .try_into()
                .expect("vk_b64 must decode to 32 bytes");
            let sig_arr: [u8; 64] = sig_bytes.as_slice().try_into().unwrap();
            let vk = VerifyingKey::from_bytes(&vk_arr).expect("vk bytes must be a valid point");
            let sig = Signature::from_bytes(&sig_arr);
            verify_pinned(&vk, &msg, &sig)
        };

        assert_eq!(
            got, v.expected,
            "vector {:?}: verify_pinned returned {got}, expected {}",
            v.name, v.expected
        );
        checked += 1;
    }
    assert_eq!(checked, file.vectors.len());
}
