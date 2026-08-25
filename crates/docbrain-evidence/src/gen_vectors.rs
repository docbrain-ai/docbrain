// SPDX-License-Identifier: MIT
//! Generator for `vectors/ed25519_pin.json` — the frozen cross-language
//! ed25519 contract (T-G). Gated behind the `gen-vectors` feature; NEVER run
//! in CI. The committed JSON is the source of truth: `tests/vectors.rs` loads
//! it and asserts `verify_pinned` returns exactly the recorded booleans. To
//! regenerate (only when the pin itself changes, which should be rare and
//! deliberate):
//!
//! ```sh
//! cargo test -p docbrain-evidence --features gen-vectors -- generate_vectors
//! ```

use curve25519_dalek::constants::EIGHT_TORSION;
use curve25519_dalek::edwards::CompressedEdwardsY;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

/// The ed25519 field prime `p = 2^255 - 19`, little-endian bytes, MSB (bit
/// 255, the point-compression sign bit) left at 0. There is no public
/// curve25519-dalek constant for this (unlike `BASEPOINT_ORDER`), so it's
/// hand-transcribed here — cross-checked at generation time by
/// `non_canonical_r_alias_decodes_to_the_torsion_point`, which independently
/// confirms decompressing `P_BYTES` actually lands on the expected point
/// rather than trusting the hex alone.
const P_BYTES: [u8; 32] = [
    0xed, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f,
];

struct RawVector {
    name: &'static str,
    vk: [u8; 32],
    msg: Vec<u8>,
    sig: [u8; 64],
    expected: bool,
}

/// Deterministic pseudo-random bytes via SHA-256 counter chaining — no `rand`
/// dependency needed for fixture data that only needs to look "generic", not
/// be cryptographically random (the vectors' properties come from their
/// construction, not from the message content).
fn deterministic_bytes(len: usize, seed: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut block: [u8; 32] = Sha256::digest([seed]).into();
    while out.len() < len {
        out.extend_from_slice(&block);
        block = Sha256::digest(block).into();
    }
    out.truncate(len);
    out
}

/// Raw (unreduced) little-endian addition of two 32-byte integers. Panics if
/// the sum overflows 256 bits — it never does here since both operands are
/// under 2^253.
fn add_le_bytes(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut carry: u16 = 0;
    for i in 0..32 {
        let sum = a[i] as u16 + b[i] as u16 + carry;
        out[i] = (sum & 0xff) as u8;
        carry = sum >> 8;
    }
    assert_eq!(carry, 0, "add_le_bytes overflowed 256 bits");
    out
}

fn build_vectors() -> Vec<RawVector> {
    let sk = SigningKey::from_bytes(&[0x11; 32]);
    let vk = sk.verifying_key();
    let vk_bytes = vk.to_bytes();

    let mut vectors = Vec::new();

    // --- 8 valid cases ------------------------------------------------
    let valid_msgs: [(&str, Vec<u8>); 8] = [
        ("valid-empty-msg", Vec::new()),
        ("valid-one-byte-msg", vec![0x42]),
        (
            "valid-ascii-msg",
            b"the quick brown fox jumps over the lazy dog".to_vec(),
        ),
        (
            "valid-opaque-bytes-msg",
            vec![0u8, 0xFF, b'\n', b'\r', b'"', 0x01, 0x1b],
        ),
        ("valid-multibyte-utf8-msg", "évidence pièce jointe 证据".as_bytes().to_vec()),
        ("valid-64-byte-msg", deterministic_bytes(64, 0x01)),
        ("valid-4096-byte-msg", deterministic_bytes(4096, 0x02)),
        ("valid-1mib-msg", deterministic_bytes(1024 * 1024, 0x03)),
    ];
    for (name, msg) in valid_msgs {
        let sig = sk.sign(&msg);
        vectors.push(RawVector {
            name,
            vk: vk_bytes,
            msg,
            sig: sig.to_bytes(),
            expected: true,
        });
    }

    // --- adversarial: non-canonical high-S signature -------------------
    // Take a genuine (R, S) signature and replace S with S + L (raw,
    // unreduced 256-bit little-endian addition). S was canonical (< L), so
    // S + L lands in [L, 2L), which is non-canonical and still fits in 32
    // bytes (L ~ 2^252.5, so S + L < 2^254). ed25519-dalek's `check_scalar`
    // (via `Scalar::from_canonical_bytes`) rejects any byte string >= L, so
    // this must fail to parse before signature math even runs — this is
    // EXACTLY the S-malleability verify_strict exists to close.
    {
        let msg = b"non-canonical S malleability probe".to_vec();
        let sig = sk.sign(&msg);
        let sig_bytes = sig.to_bytes();
        let mut r = [0u8; 32];
        r.copy_from_slice(&sig_bytes[0..32]);
        let mut s = [0u8; 32];
        s.copy_from_slice(&sig_bytes[32..64]);
        // `BASEPOINT_ORDER` is marked deprecated upstream ("should not have
        // been in public API") but remains the documented, exact value of
        // L — the alternative is hand-transcribing the constant, which is
        // strictly more error-prone for a value this security-sensitive.
        #[allow(deprecated)]
        let l_bytes = curve25519_dalek::constants::BASEPOINT_ORDER.to_bytes();
        let s_noncanonical = add_le_bytes(s, l_bytes);
        let mut forged = [0u8; 64];
        forged[0..32].copy_from_slice(&r);
        forged[32..64].copy_from_slice(&s_noncanonical);
        vectors.push(RawVector {
            name: "invalid-non-canonical-high-s",
            vk: vk_bytes,
            msg,
            sig: forged,
            expected: false,
        });
    }

    // --- adversarial: R replaced by a small-order (8-torsion) point ----
    // Take a genuine signature and swap R for one of the eight documented
    // small-order points (curve25519_dalek::constants::EIGHT_TORSION),
    // leaving S untouched. verify_strict explicitly rejects any signature
    // whose R decompresses to a small-order point
    // (`signature_R.is_small_order()`) — this is the check that closes the
    // R-substitution malleability that a cofactored-only verifier misses.
    {
        let msg = b"small-order R substitution probe".to_vec();
        let sig = sk.sign(&msg);
        let sig_bytes = sig.to_bytes();
        let torsion_r = EIGHT_TORSION[1].compress().to_bytes();
        let mut forged = [0u8; 64];
        forged[0..32].copy_from_slice(&torsion_r);
        forged[32..64].copy_from_slice(&sig_bytes[32..64]);
        vectors.push(RawVector {
            name: "invalid-small-order-r",
            vk: vk_bytes,
            msg,
            sig: forged,
            expected: false,
        });
    }

    // --- adversarial: wrong key ------------------------------------------
    {
        let other_sk = SigningKey::from_bytes(&[0x22; 32]);
        let msg = b"signed by a different key entirely".to_vec();
        let sig = other_sk.sign(&msg);
        vectors.push(RawVector {
            name: "invalid-wrong-key",
            vk: vk_bytes, // verifying against sk's key, but signed by other_sk
            msg,
            sig: sig.to_bytes(),
            expected: false,
        });
    }

    // --- adversarial: small-order public key (THE canonical ZIP215 case) --
    // Take a genuine signature and swap the VERIFYING KEY for one of the
    // eight documented small-order points, leaving R and S untouched.
    // `verify_strict`'s `self.point.is_small_order()` check fires
    // unconditionally, before the verification equation is even computed
    // (confirmed by reading ed25519-dalek 2.2.0's verifying.rs) — so this
    // doesn't need a self-consistent forgery: ANY genuine (msg, R, S) paired
    // with a small-order key is rejected purely on the key's own encoding.
    // Torsion index 3 (order 8) is used here — deliberately different from
    // index 1 (used for `invalid-small-order-r` above) and index 0/2 (used
    // below) so no two vectors share a torsion index, which would read as a
    // copy-paste accident on review.
    {
        let msg = b"small-order public key probe".to_vec();
        let sig = sk.sign(&msg);
        let torsion_vk = EIGHT_TORSION[3].compress().to_bytes();
        vectors.push(RawVector {
            name: "invalid-small-order-public-key",
            vk: torsion_vk,
            msg,
            sig: sig.to_bytes(),
            expected: false,
        });
    }

    // --- adversarial: S = 0 ------------------------------------------------
    // Take a genuine signature and zero the S scalar, leaving R and the key
    // untouched. S = 0 is canonical (0 < L), so this reaches the actual
    // verification equation (unlike the non-canonical-S vector above, which
    // is rejected at parse time) — it's rejected because a genuine R was
    // computed for a different, real S, and `expected_R = [0]B - [k]A`
    // essentially never coincides with that R. This is NOT a self-consistent
    // forgery (see report); it's a degenerate-scalar boundary case, tested
    // because some implementations special-case zero/identity scalars in
    // ways that can accidentally short-circuit past a check.
    {
        let msg = b"S equals zero probe".to_vec();
        let sig = sk.sign(&msg);
        let sig_bytes = sig.to_bytes();
        let mut forged = [0u8; 64];
        forged[0..32].copy_from_slice(&sig_bytes[0..32]);
        // forged[32..64] left as [0u8; 64]'s default zero-fill: S = 0.
        vectors.push(RawVector {
            name: "invalid-s-equals-zero",
            vk: vk_bytes,
            msg,
            sig: forged,
            expected: false,
        });
    }

    // --- adversarial: R = identity -----------------------------------------
    // Take a genuine signature and swap R for the identity point's
    // compressed encoding (`EIGHT_TORSION[0]`), leaving S and the key
    // untouched. Same unconditional `signature_R.is_small_order()` gate as
    // `invalid-small-order-r` above (identity trivially has order 1), using
    // torsion index 0 specifically — the maximally-degenerate case, and
    // notably the ONE self-consistent forgery in this file that would
    // satisfy the base verification equation without the small-order guard:
    // for r=0 (so R=identity) and ANY key/scalar, `S = k*a mod L` makes
    // `[S]B - [k]A = [k*a]B - [k]A = identity = R` hold exactly. This vector
    // doesn't bother constructing that self-consistent S (S is just copied
    // from the genuine signature, R swapped) because the is_small_order(R)
    // check fires first regardless — but it's worth recording that this
    // specific case COULD be made "would pass a naive equation check" too.
    {
        let msg = b"R equals identity probe".to_vec();
        let sig = sk.sign(&msg);
        let sig_bytes = sig.to_bytes();
        let identity_r = EIGHT_TORSION[0].compress().to_bytes();
        let mut forged = [0u8; 64];
        forged[0..32].copy_from_slice(&identity_r);
        forged[32..64].copy_from_slice(&sig_bytes[32..64]);
        vectors.push(RawVector {
            name: "invalid-r-identity",
            vk: vk_bytes,
            msg,
            sig: forged,
            expected: false,
        });
    }

    // --- adversarial: small-order R, non-canonically encoded ---------------
    // `EIGHT_TORSION[2]` (order 4) has canonical y = 0 — one of only two
    // torsion points (besides the identity) whose canonical y is small
    // enough (< 19) to have a representable non-canonical alias y' = y + p
    // within the 255-bit encoding. This is a 4th small-order-R variant, NOT
    // the classical ZIP215 test (a LARGE-order point re-encoded
    // non-canonically, in a signature that would otherwise verify):
    // constructing THAT requires either the discrete log of a chosen
    // large-order point, or a ~19-in-2^255 search over nonces to land one on
    // a small-y point by chance — confirmed infeasible, and confirmed to not
    // even exist as a constructible case for this curve (see
    // `invalid-low-order-residue-r` below and the fix report: an exhaustive
    // scan of the published CCTV ed25519 corpus, 914 vectors, found zero
    // vectors combining `non_canonical_R` without `low_order_R`). What THIS
    // vector tests: `is_small_order()` correctly recognizes a small-order
    // point via its NON-CANONICAL encoding too — confirmed to be exactly
    // the `is_small_order(R)` early-exit that fires (not a fallback byte
    // comparison — verify_strict's source was read to confirm this is the
    // ONLY guard active for a pure-torsion R). `non_canonical_r_alias_...`
    // below independently verifies the construction lands on the claimed
    // point before trusting it in a vector.
    {
        let msg = b"non-canonical R encoding probe".to_vec();
        let sig = sk.sign(&msg);
        let sig_bytes = sig.to_bytes();
        let canonical_r = EIGHT_TORSION[2].compress().to_bytes();
        let sign_bit = canonical_r[31] & 0x80;
        let mut noncanonical_r = P_BYTES; // y' = p (canonical y was 0; p = 0 + p)
        noncanonical_r[31] |= sign_bit;
        let mut forged = [0u8; 64];
        forged[0..32].copy_from_slice(&noncanonical_r);
        forged[32..64].copy_from_slice(&sig_bytes[32..64]);
        vectors.push(RawVector {
            name: "invalid-small-order-r-alt-encoding",
            vk: vk_bytes,
            msg,
            sig: forged,
            expected: false,
        });
    }

    // --- adversarial: low-order residue in R (the genuine ZIP215/large-order
    //     case, SOURCED, not self-constructed) ------------------------------
    // Sourced from the published CCTV ed25519 test-vector corpus:
    // https://github.com/C2SP/CCTV/blob/main/ed25519/ed25519vectors.json
    // (an extension of novifinancial/ed25519-speccheck, itself companion
    // data to "Taming the many EdDSAs", https://eprint.iacr.org/2020/823,
    // and hdevalence's https://hdevalence.ca/blog/2020-10-04-its-25519am).
    // Vector #267: key = pubkey A (a completely ORDINARY large-order key —
    // the SAME key value appears elsewhere in the corpus with NO flags at
    // all, confirming it isn't itself special), sig = R||S where R has a
    // `low_order_component` (a low-order point ADDED to a genuine
    // large-order component — NOT purely small-order, so
    // `is_small_order(R)` is FALSE and does not fire), flagged
    // `low_order_residue`: R's low-order component and [k]A's low-order
    // component do NOT cancel, so a COFACTORED verification formula
    // (multiplying both sides by 8, as ZIP215 and many "permissive"
    // implementations do) ACCEPTS this signature, while `verify_strict`'s
    // direct, non-cofactored, byte-exact-R-comparison formula REJECTS it —
    // confirmed by actually running it through `verify_pinned` (see the fix
    // report), not assumed from the flag alone.
    //
    // This is embedded in place of a literal "non-canonical encoding of a
    // large-order R" because that case does not exist for this curve (see
    // above) — but this IS the genuine cross-language risk that motivated
    // the request: a case where two "conformant" ed25519 verifiers, one
    // cofactored and one not, disagree. It is in fact the eponymous
    // Zcash/ZIP215 concern referenced in the project's own design spec.
    {
        fn hex32(s: &str) -> [u8; 32] {
            let v: Vec<u8> = (0..64)
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect();
            v.try_into().unwrap()
        }
        fn hex64(s: &str) -> [u8; 64] {
            let v: Vec<u8> = (0..128)
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect();
            v.try_into().unwrap()
        }
        let cctv_key = hex32("ef75b20e7540e3dff77404193652ba2bd13df99c1508eee1515e27ae25f28076");
        let cctv_sig = hex64(
            "36684ea91032ba5b1dbab2d02f4debc74c3327f2b3802e2e4d371aa42b12b56b2d471882773a677af1e3824e757a33f8ddf7bbeaf3d28a09595eb8daa0d74a03",
        );
        vectors.push(RawVector {
            name: "invalid-low-order-residue-r",
            vk: cctv_key,
            msg: b"ed25519vectors 3".to_vec(),
            sig: cctv_sig,
            expected: false,
        });
    }

    // Note: "truncated signature" is deliberately NOT built here — its
    // whole point is that the truncated bytes DON'T fit `[u8; 64]`, so it
    // can't be represented as a `RawVector`/`Signature` at all. It's added
    // directly to the JSON output in `generate_vectors` below, and is
    // exercised in `tests/vectors.rs` by the same length gate a real
    // envelope parser applies before it can even construct a `Signature`.
    vectors
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use ed25519_dalek::{Signature, VerifyingKey};
    use serde::Serialize;
    use std::path::PathBuf;

    #[derive(Serialize)]
    struct JsonVector {
        name: String,
        vk_b64: String,
        msg_b64: String,
        sig_b64: String,
        expected: bool,
    }

    #[derive(Serialize)]
    struct VectorFile {
        pin: String,
        vectors: Vec<JsonVector>,
    }

    #[test]
    fn generate_vectors() {
        let sk = SigningKey::from_bytes(&[0x11; 32]);
        let vk_bytes = sk.verifying_key().to_bytes();
        let mut vectors = build_vectors();

        // truncated-sig vector, built directly here (kept out of
        // `build_vectors` since its length is deliberately NOT 64 bytes and
        // doesn't fit the `[u8; 64]` RawVector shape).
        let trunc_msg = b"truncated signature probe".to_vec();
        let trunc_sig_full = sk.sign(&trunc_msg).to_bytes();
        let trunc_sig_63 = trunc_sig_full[..63].to_vec();

        let mut json_vectors: Vec<JsonVector> = vectors
            .drain(..)
            .map(|v| JsonVector {
                name: v.name.to_string(),
                vk_b64: STANDARD.encode(v.vk),
                msg_b64: STANDARD.encode(&v.msg),
                sig_b64: STANDARD.encode(v.sig),
                expected: v.expected,
            })
            .collect();
        json_vectors.push(JsonVector {
            name: "invalid-truncated-sig".to_string(),
            vk_b64: STANDARD.encode(vk_bytes),
            msg_b64: STANDARD.encode(&trunc_msg),
            sig_b64: STANDARD.encode(&trunc_sig_63),
            expected: false,
        });

        let file = VectorFile {
            pin: crate::ed25519_pin::PIN.to_string(),
            vectors: json_vectors,
        };

        let json = serde_json::to_string_pretty(&file).unwrap();
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("vectors");
        path.push("ed25519_pin.json");
        std::fs::write(&path, json).expect("write vectors/ed25519_pin.json");
        println!("wrote {} vectors to {}", file.vectors.len(), path.display());
    }

    /// Sanity: every generated vector actually round-trips through
    /// `verify_pinned` and matches its recorded `expected` — proves the
    /// generator isn't producing vectors that don't exercise what their
    /// name claims (checked here at generation time, not just by the
    /// frozen-file loader in tests/vectors.rs).
    #[test]
    fn generated_vectors_match_verify_pinned() {
        for v in build_vectors() {
            let vk = VerifyingKey::from_bytes(&v.vk).expect("valid vk bytes");
            let sig = Signature::from_bytes(&v.sig);
            let got = crate::verify_pinned(&vk, &v.msg, &sig);
            assert_eq!(got, v.expected, "vector {} mismatched", v.name);
        }
    }

    /// Independent proof for the `invalid-small-order-r-alt-encoding` vector's
    /// load-bearing claim: that `P_BYTES` (y' = p, with `EIGHT_TORSION[2]`'s
    /// sign bit) decompresses to the SAME point as `EIGHT_TORSION[2]`'s own
    /// canonical encoding (y = 0), not some other point or a decode failure.
    /// Equality is checked via re-compression (canonical bytes), not struct
    /// equality on `EdwardsPoint` — the two decompressions likely hold Y in
    /// different (non-reduced vs. reduced) internal limb representations
    /// even though they denote the same field element, so only the
    /// normalizing round-trip through `.compress()` is a valid equality
    /// check here.
    #[test]
    fn non_canonical_r_alias_decodes_to_the_claimed_torsion_point() {
        let canonical = EIGHT_TORSION[2].compress().to_bytes();
        let sign_bit = canonical[31] & 0x80;
        let mut noncanonical = P_BYTES;
        noncanonical[31] |= sign_bit;
        assert_ne!(
            noncanonical, canonical,
            "the non-canonical and canonical encodings must actually differ as bytes"
        );

        let decoded = CompressedEdwardsY(noncanonical)
            .decompress()
            .expect("non-canonical y=p must still decompress (permissive decode)");
        assert!(
            decoded.is_small_order(),
            "the point named by y=p must be recognized as small-order, same as y=0"
        );
        assert_eq!(
            decoded.compress().to_bytes(),
            canonical,
            "y=p and y=0 must decompress to the identical point (mod-p equivalence)"
        );
    }
}
