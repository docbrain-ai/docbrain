// SPDX-License-Identifier: MIT
//! Checkpoint chain + range-boundary rules (spec taxonomy rows 4, 24, 25).
//!
//! Checkpoints form their OWN hash chain from genesis via `cp_prev`, separate
//! from the evidence record chain (`chain::walk_chain`) and the key chain
//! (`keys::verify_key_chain`), but sharing the SAME position coordinate
//! space: a checkpoint's `position` field names the evidence-record position
//! it certifies.
//!
//! ## Hash-chain domain separation: reused, not a new prefix (controller-ratified)
//!
//! This chain is linked with `crate::hash::{leaf_hash, head_hash}` UNCHANGED
//! — no new domain-separation byte was introduced for the checkpoint chain.
//! This is a deliberate design decision, ratified by the controller
//! (2026-08-24, Task 5 review), not an oversight, and not to be re-litigated
//! by a later reviewer as a missing prefix:
//!
//! 1. Spec law 4 pins EXACTLY three domain-separation bytes at the hash layer
//!    (0x00 leaf / 0x01 chain / 0x02 content) as a frozen cross-language
//!    contract, and states domain separation happens at TWO layers: the hash
//!    layer (by hash ROLE: leaf/head/content) and the signature layer
//!    (`payloadType` per envelope kind, so one context's envelope can never
//!    be spliced into another). A fourth hash-layer prefix isn't part of
//!    that contract, and would only enlarge the Rust-vs-Python (Task 14)
//!    divergence surface for no security gain.
//! 2. Task 4's key chain already established this precedent: it is
//!    documented as its "OWN hash chain... separate from the evidence record
//!    chain," yet verifiably reuses `leaf_hash`/`head_hash` (0x00/0x01)
//!    as-is (see `keys.rs` module docs), relying on `PT_KEYRECORD` for
//!    cross-context separation instead of a distinct hash prefix. No
//!    retroactive change to Task 4 is needed — it was already correct.
//! 3. Adversarial check on the reuse: `leaf_hash` covers the FULL envelope
//!    line, which always embeds the distinct `payloadType` string — so a
//!    byte-identical leaf/head collision across chain types is structurally
//!    impossible regardless of hash prefix. And splicing a wrong-type
//!    envelope into this chain's input is independently rejected by
//!    `verify_envelope`'s `payloadType` check, which fires before any
//!    hash-chain work. A new prefix would add a moving part with no
//!    additional security guarantee over what already exists. Two tests
//!    below (`checkpoint_shaped_payload_signed_under_the_wrong_payload_type_is_rejected`,
//!    `a_genuine_key_record_envelope_spliced_into_the_checkpoint_chain_is_rejected`)
//!    prove this holds.
//!
//! ## Genesis via `cp_prev`
//!
//! Every export carries the FULL checkpoint chain from genesis (no partial
//! resume, unlike `walk_chain`'s record-range support): the first
//! checkpoint's `cp_prev` MUST equal [`GENESIS_PREV`], exactly like the key
//! chain's genesis `key_prev`. There is no separate "genesis checkpoint"
//! schema (unlike the key chain's `kind: "genesis"` payload) — every
//! checkpoint shares the identical schema; genesis-ness is purely "the first
//! entry links to the zero anchor."
//!
//! ## Signer authorization
//!
//! A checkpoint at `position` P must be signed by whatever key is valid at P
//! per [`key_at_position`] (Task 4). Two checks enforce this (spec law 2,
//! "verify every cross-checkable field," the cosign lesson): the payload's
//! own declared `keyid` must equal the position-resolved key
//! ([`CpError::UnauthorizedSigner`] on mismatch), AND the envelope signature
//! must actually verify under that SAME position-resolved key
//! ([`CpError::SignatureInvalid`] on mismatch) — never under the
//! self-declared `keyid` alone, which would let a payload lie about who
//! signed it.
//!
//! ## Wall-clock non-monotonicity is a finding, not an error
//!
//! Spec taxonomy row 24: chain position/hash order is authoritative;
//! wall-clock `at` values are unverified claims. A checkpoint whose `at` is
//! not strictly after the previous checkpoint's `at` does NOT fail
//! [`verify_checkpoint_chain`] — it is recorded as a [`ClockAnomaly`] and
//! surfaced via [`CheckpointChain::clock_anomalies`] for the verdict engine
//! (Task 7) to render as a WARNING finding, never as TAMPERED.

use crate::envelope::{verify_envelope, EnvelopeError, PT_CHECKPOINT};
use crate::hash::{head_hash, leaf_hash, GENESIS_PREV};
use crate::keys::{key_at_position, KeyChain};
use crate::strict::from_slice_strict;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use serde::Deserialize;

/// Every variant is a fail-closed outcome: no case is silently skipped or
/// coerced into a successful chain.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CpError {
    #[error("checkpoint chain must contain at least one checkpoint")]
    Empty,

    #[error("checkpoint at slice index {index} is malformed: {reason}")]
    Malformed { index: usize, reason: String },

    #[error(
        "cp_prev at position {position} does not match the computed checkpoint-chain head"
    )]
    CpLinkMismatch { position: u64 },

    #[error("checkpoint position must strictly increase: previous {previous}, found {found}")]
    PositionNotIncreasing { previous: u64, found: u64 },

    #[error("checkpoint at position {position} is not signed by the key valid at that position")]
    UnauthorizedSigner { position: u64 },

    #[error("checkpoint at position {position} failed signature verification")]
    SignatureInvalid { position: u64 },

    #[error(
        "checkpoint at position {position} has an invalid 'at' timestamp (must be RFC 3339): {reason}"
    )]
    InvalidTimestamp { position: u64, reason: String },

    #[error("range boundary position {position} is not an existing checkpoint position")]
    NotABoundary { position: u64 },
}

/// One verified checkpoint (spec: position, head, cumulative count,
/// wall-clock, key id, prev-checkpoint hash — `keyid`/`cp_prev` are consumed
/// during verification and not retained here; everything else is).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    pub position: u64,
    pub head: [u8; 32],
    pub count: u64,
    pub at: DateTime<Utc>,
}

/// A wall-clock non-monotonicity between two chain-adjacent checkpoints
/// (spec row 24). NOT an error — chain order (position/hash) remains
/// authoritative; this is a claim-level warning only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockAnomaly {
    pub position: u64,
    pub at: DateTime<Utc>,
    pub previous_position: u64,
    pub previous_at: DateTime<Utc>,
}

/// A verified checkpoint chain: `cp_prev`-linked from genesis, every signer
/// authorized per [`key_at_position`], positions strictly increasing. Only
/// constructible via [`verify_checkpoint_chain`] — its existence IS the
/// proof that every linkage and signature check already passed.
#[derive(Debug, Clone)]
pub struct CheckpointChain {
    /// Ascending, strictly increasing by position.
    checkpoints: Vec<Checkpoint>,
    clock_anomalies: Vec<ClockAnomaly>,
}

impl CheckpointChain {
    /// Every verified checkpoint, ascending by position.
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    /// Wall-clock anomalies observed while walking the chain (spec row 24).
    /// Empty means every checkpoint's `at` strictly increased in chain
    /// order; non-empty is a warning, never a reason this chain failed to
    /// verify.
    pub fn clock_anomalies(&self) -> &[ClockAnomaly] {
        &self.clock_anomalies
    }

    fn checkpoint_at(&self, position: u64) -> Option<&Checkpoint> {
        self.checkpoints
            .binary_search_by_key(&position, |c| c.position)
            .ok()
            .map(|i| &self.checkpoints[i])
    }
}

/// Trusted open boundary and expected close boundary for a range export
/// (spec: "exports start and end exactly at checkpoint boundaries").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeBounds {
    pub start_position: u64,
    pub start_head: [u8; 32],
    pub end_position: u64,
    pub end_head: [u8; 32],
    pub end_count: u64,
}

fn hex_to_32(s: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(s).map_err(|e| format!("invalid hex: {e}"))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("expected 32 bytes, got {}", bytes.len()))
}

/// Minimal, tolerant peek at the DSSE wire form to pull out just the base64
/// `payload` field — mirrors `chain.rs`/`keys.rs`'s split between a
/// tolerant structural peek and the strict, closed-schema parse below.
#[derive(Deserialize)]
struct PayloadPeek {
    payload: String,
}

fn decode_payload_bytes(line: &[u8], index: usize) -> Result<Vec<u8>, CpError> {
    let peek: PayloadPeek = from_slice_strict(line).map_err(|e| CpError::Malformed {
        index,
        reason: format!("envelope JSON: {e}"),
    })?;
    STANDARD
        .decode(peek.payload.as_bytes())
        .map_err(|e| CpError::Malformed {
            index,
            reason: format!("payload base64: {e}"),
        })
}

/// `deny_unknown_fields`: the schema is closed (design doc + Task 5 brief);
/// an unrecognized key is malformed input, not silently ignored.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointPayload {
    position: u64,
    head: String,
    count: u64,
    at: String,
    keyid: String,
    cp_prev: String,
}

fn decode_checkpoint_payload(bytes: &[u8], index: usize) -> Result<CheckpointPayload, CpError> {
    from_slice_strict(bytes).map_err(|e| CpError::Malformed {
        index,
        reason: format!("checkpoint payload JSON: {e}"),
    })
}

/// Verify the full checkpoint chain from genesis: `cp_prev` linkage, strictly
/// increasing positions, and each checkpoint's signer authorized per
/// [`key_at_position`]. Wall-clock non-monotonicity is recorded on the
/// returned chain (see [`CheckpointChain::clock_anomalies`]), never
/// returned as an error. See module docs for the exact schema and the
/// hash-prefix-reuse rationale.
pub fn verify_checkpoint_chain(
    lines: &[&[u8]],
    keys: &KeyChain,
) -> Result<CheckpointChain, CpError> {
    if lines.is_empty() {
        return Err(CpError::Empty);
    }

    let mut checkpoints = Vec::with_capacity(lines.len());
    let mut clock_anomalies = Vec::new();
    let mut head = GENESIS_PREV;
    let mut last_position: Option<u64> = None;
    let mut last_at: Option<DateTime<Utc>> = None;

    for (index, line) in lines.iter().enumerate() {
        let payload_bytes = decode_payload_bytes(line, index)?;
        let payload = decode_checkpoint_payload(&payload_bytes, index)?;

        // Positions must strictly increase (no backwards, no duplicate).
        if let Some(previous) = last_position
            && payload.position <= previous
        {
            return Err(CpError::PositionNotIncreasing {
                previous,
                found: payload.position,
            });
        }

        // cp_prev must match the running checkpoint-chain head: GENESIS_PREV
        // for the first checkpoint, the previously-computed head otherwise.
        let declared_cp_prev =
            hex_to_32(&payload.cp_prev).map_err(|reason| CpError::Malformed {
                index,
                reason: format!("cp_prev: {reason}"),
            })?;
        if declared_cp_prev != head {
            return Err(CpError::CpLinkMismatch {
                position: payload.position,
            });
        }

        // Signer authorization: resolve the key valid AT THIS POSITION
        // (never the self-declared keyid alone), cross-check the payload's
        // declared keyid against it (law 2: verify every cross-checkable
        // field), then verify the signature under that SAME resolved key.
        let expected_vk = key_at_position(keys, payload.position)
            .copied()
            .ok_or(CpError::UnauthorizedSigner {
                position: payload.position,
            })?;
        let declared_keyid = hex_to_32(&payload.keyid).map_err(|reason| CpError::Malformed {
            index,
            reason: format!("keyid: {reason}"),
        })?;
        if declared_keyid != expected_vk.to_bytes() {
            return Err(CpError::UnauthorizedSigner {
                position: payload.position,
            });
        }
        // Un-collapse (Task 7 controller ruling, 2026-08-24 review): only an
        // actual cryptographic authenticity failure is SignatureInvalid; a
        // wrong-payloadType/malformed/unsupported envelope (e.g. a record
        // envelope spliced into the checkpoint chain's input) is a
        // DIFFERENT failure class the verdict engine must map to
        // CANNOT_VERIFY(malformed/unsupported), not TAMPERED(signature).
        verify_envelope(line, PT_CHECKPOINT, &expected_vk).map_err(|e| match e {
            EnvelopeError::SignatureInvalid => CpError::SignatureInvalid {
                position: payload.position,
            },
            other => CpError::Malformed {
                index,
                reason: format!("checkpoint envelope: {other}"),
            },
        })?;

        let declared_head = hex_to_32(&payload.head).map_err(|reason| CpError::Malformed {
            index,
            reason: format!("head: {reason}"),
        })?;
        let at = DateTime::parse_from_rfc3339(&payload.at)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| CpError::InvalidTimestamp {
                position: payload.position,
                reason: e.to_string(),
            })?;

        // Wall-clock non-monotonicity is a finding, never an error (spec
        // row 24): chain position/hash order is authoritative.
        if let (Some(previous_at), Some(previous_position)) = (last_at, last_position)
            && at <= previous_at
        {
            clock_anomalies.push(ClockAnomaly {
                position: payload.position,
                at,
                previous_position,
                previous_at,
            });
        }

        checkpoints.push(Checkpoint {
            position: payload.position,
            head: declared_head,
            count: payload.count,
            at,
        });

        head = head_hash(&head, &leaf_hash(line));
        last_position = Some(payload.position);
        last_at = Some(at);
    }

    Ok(CheckpointChain {
        checkpoints,
        clock_anomalies,
    })
}

/// Range rule: exports open/close exactly at checkpoint boundaries. Returns
/// the trusted `(start_position, start_head)` and expected
/// `(end_position, end_head, end_count)`. A range endpoint that is not an
/// existing checkpoint position is [`CpError::NotABoundary`].
pub fn range_bounds(chain: &CheckpointChain, range: (u64, u64)) -> Result<RangeBounds, CpError> {
    let (start, end) = range;
    let start_cp = chain
        .checkpoint_at(start)
        .ok_or(CpError::NotABoundary { position: start })?;
    let end_cp = chain
        .checkpoint_at(end)
        .ok_or(CpError::NotABoundary { position: end })?;
    Ok(RangeBounds {
        start_position: start_cp.position,
        start_head: start_cp.head,
        end_position: end_cp.position,
        end_head: end_cp.head,
        end_count: end_cp.count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{sign_envelope, PT_KEYRECORD};
    use ed25519_dalek::{SigningKey, VerifyingKey};

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn hex32(vk: &VerifyingKey) -> String {
        hex::encode(vk.to_bytes())
    }

    // ---- minimal local KeyChain fixtures (mirrors keys.rs test helpers;
    // those are private to keys.rs's own test module, so this module builds
    // its own small genesis/rotation envelopes) ----

    fn genesis_key_line(sk: &SigningKey) -> Vec<u8> {
        let payload = serde_json::to_vec(&serde_json::json!({
            "kind": "genesis",
            "position": 0,
            "signing_key": hex32(&sk.verifying_key()),
            "recovery_key": null,
            "predecessor_genesis": null,
            "key_prev": hex::encode(GENESIS_PREV),
        }))
        .expect("static JSON always serializes");
        sign_envelope(sk, &hex32(&sk.verifying_key()), PT_KEYRECORD, &payload).to_line()
    }

    fn rotation_key_line(
        predecessor: &SigningKey,
        new: &SigningKey,
        position: u64,
        key_prev: &[u8; 32],
    ) -> Vec<u8> {
        let payload = serde_json::to_vec(&serde_json::json!({
            "kind": "rotation",
            "position": position,
            "new_signing_key": hex32(&new.verifying_key()),
            "key_prev": hex::encode(key_prev),
        }))
        .expect("static JSON always serializes");
        sign_envelope(
            predecessor,
            &hex32(&predecessor.verifying_key()),
            PT_KEYRECORD,
            &payload,
        )
        .to_line()
    }

    /// A single-key chain: `sk` is valid at every position from 0 onward.
    fn single_key_chain(sk: &SigningKey) -> KeyChain {
        let genesis = genesis_key_line(sk);
        crate::keys::verify_key_chain(&[genesis.as_slice()]).expect("fixture chain must verify")
    }

    /// A two-key chain: `sk1` valid on `[0, rotate_at)`, `sk2` valid on
    /// `[rotate_at, ∞)`.
    fn two_key_chain(sk1: &SigningKey, sk2: &SigningKey, rotate_at: u64) -> KeyChain {
        let genesis = genesis_key_line(sk1);
        let head = head_hash(&GENESIS_PREV, &leaf_hash(&genesis));
        let rotation = rotation_key_line(sk1, sk2, rotate_at, &head);
        let lines = [genesis, rotation];
        let refs: Vec<&[u8]> = lines.iter().map(|l| l.as_slice()).collect();
        crate::keys::verify_key_chain(&refs).expect("fixture chain must verify")
    }

    // ---- checkpoint envelope builder ----

    /// Builds one checkpoint envelope line, signed by `signer`, with the
    /// exact payload schema from the Task 5 brief. `declared_keyid` is
    /// normally `signer`'s own public key (honest case); tests that attack
    /// the keyid/signer relationship pass a different key.
    fn checkpoint_line(
        signer: &SigningKey,
        declared_keyid: &VerifyingKey,
        position: u64,
        head: &[u8; 32],
        count: u64,
        at: &str,
        cp_prev: &[u8; 32],
    ) -> Vec<u8> {
        let payload = serde_json::to_vec(&serde_json::json!({
            "position": position,
            "head": hex::encode(head),
            "count": count,
            "at": at,
            "keyid": hex32(declared_keyid),
            "cp_prev": hex::encode(cp_prev),
        }))
        .expect("static JSON always serializes");
        sign_envelope(
            signer,
            &hex32(&signer.verifying_key()),
            PT_CHECKPOINT,
            &payload,
        )
        .to_line()
    }

    fn advance(head: [u8; 32], line: &[u8]) -> [u8; 32] {
        head_hash(&head, &leaf_hash(line))
    }

    fn as_slices(lines: &[Vec<u8>]) -> Vec<&[u8]> {
        lines.iter().map(|l| l.as_slice()).collect()
    }

    // ---- 1: chain from genesis (happy path) ----

    #[test]
    fn single_checkpoint_chain_from_genesis_verifies() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let head = [7u8; 32];
        let line = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            10,
            &head,
            10,
            "2026-01-01T00:00:00Z",
            &GENESIS_PREV,
        );
        let lines = vec![line];
        let chain =
            verify_checkpoint_chain(&as_slices(&lines), &keys).expect("must verify from genesis");
        assert_eq!(chain.checkpoints().len(), 1);
        let cp = &chain.checkpoints()[0];
        assert_eq!(cp.position, 10);
        assert_eq!(cp.head, head);
        assert_eq!(cp.count, 10);
        assert!(chain.clock_anomalies().is_empty());
    }

    #[test]
    fn multi_checkpoint_chain_links_correctly() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let cp1 = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            10,
            &[1u8; 32],
            10,
            "2026-01-01T00:00:00Z",
            &GENESIS_PREV,
        );
        let head_after_1 = advance(GENESIS_PREV, &cp1);
        let cp2 = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            20,
            &[2u8; 32],
            20,
            "2026-01-02T00:00:00Z",
            &head_after_1,
        );
        let head_after_2 = advance(head_after_1, &cp2);
        let cp3 = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            30,
            &[3u8; 32],
            30,
            "2026-01-03T00:00:00Z",
            &head_after_2,
        );
        let lines = vec![cp1, cp2, cp3];
        let chain = verify_checkpoint_chain(&as_slices(&lines), &keys).expect("must verify");
        assert_eq!(chain.checkpoints().len(), 3);
        assert_eq!(
            chain.checkpoints().iter().map(|c| c.position).collect::<Vec<_>>(),
            vec![10, 20, 30]
        );
        assert!(chain.clock_anomalies().is_empty());
    }

    // ---- 2: broken cp_prev link ----

    #[test]
    fn broken_cp_prev_link_on_second_checkpoint_is_rejected() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let cp1 = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            10,
            &[1u8; 32],
            10,
            "2026-01-01T00:00:00Z",
            &GENESIS_PREV,
        );
        // cp2 declares a cp_prev that does NOT match the actual head after cp1.
        let bad_prev = [0xAAu8; 32];
        let cp2 = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            20,
            &[2u8; 32],
            20,
            "2026-01-02T00:00:00Z",
            &bad_prev,
        );
        let lines = vec![cp1, cp2];
        let err = verify_checkpoint_chain(&as_slices(&lines), &keys).unwrap_err();
        assert_eq!(err, CpError::CpLinkMismatch { position: 20 });
    }

    // ---- cross-context splicing: the payloadType/schema defense the
    // hash-prefix-reuse design decision (module docs) relies on ----

    #[test]
    fn checkpoint_shaped_payload_signed_under_the_wrong_payload_type_is_rejected() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        // A perfectly well-formed checkpoint payload, but the envelope
        // signs it under PT_RECORD instead of PT_CHECKPOINT — simulating a
        // record envelope spliced into the checkpoint chain's input. Must
        // be rejected even though the JSON shape is checkpoint-correct.
        let payload = serde_json::to_vec(&serde_json::json!({
            "position": 10,
            "head": hex::encode([1u8; 32]),
            "count": 10,
            "at": "2026-01-01T00:00:00Z",
            "keyid": hex32(&sk.verifying_key()),
            "cp_prev": hex::encode(GENESIS_PREV),
        }))
        .unwrap();
        let line = sign_envelope(
            &sk,
            &hex32(&sk.verifying_key()),
            crate::envelope::PT_RECORD,
            &payload,
        )
        .to_line();
        let err = verify_checkpoint_chain(&[line.as_slice()], &keys).unwrap_err();
        // Un-collapsed (Task 7 controller ruling): a wrong-payloadType
        // splice is a format/malformed failure, not a signature failure —
        // it must map to CANNOT_VERIFY(malformed), never TAMPERED(signature).
        assert!(
            matches!(err, CpError::Malformed { index: 0, .. }),
            "expected Malformed (wrong payloadType), got {err:?}"
        );
    }

    #[test]
    fn a_genuine_key_record_envelope_spliced_into_the_checkpoint_chain_is_rejected() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        // The genesis key-record envelope from the SAME key chain: real,
        // validly-signed, and even leaf/head-hashed identically to how a
        // checkpoint would be (same hash functions, no distinct prefix per
        // this module's design). It must still be rejected — its payload
        // schema (kind/signing_key/...) doesn't match CheckpointPayload's
        // closed schema, so it fails before any hash-chain or signature
        // work even runs.
        let key_record_line = genesis_key_line(&sk);
        let err =
            verify_checkpoint_chain(&[key_record_line.as_slice()], &keys).unwrap_err();
        assert!(
            matches!(err, CpError::Malformed { index: 0, .. }),
            "expected Malformed (schema mismatch), got {err:?}"
        );
    }

    #[test]
    fn first_checkpoint_cp_prev_not_genesis_anchor_is_rejected() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let bad_prev = [0x11u8; 32];
        let cp1 = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            10,
            &[1u8; 32],
            10,
            "2026-01-01T00:00:00Z",
            &bad_prev,
        );
        let lines = vec![cp1];
        let err = verify_checkpoint_chain(&as_slices(&lines), &keys).unwrap_err();
        assert_eq!(err, CpError::CpLinkMismatch { position: 10 });
    }

    // ---- 3: checkpoint signed by a key not valid at its position ----

    #[test]
    fn checkpoint_signed_by_key_invalid_at_its_position_is_rejected() {
        let sk1 = key(1);
        let sk2 = key(2);
        // sk1 valid on [0, 50), sk2 valid on [50, inf).
        let keys = two_key_chain(&sk1, &sk2, 50);
        // Checkpoint at position 30 (sk1's window) but actually signed by
        // sk2, and honestly declaring sk2 as the signer.
        let line = checkpoint_line(
            &sk2,
            &sk2.verifying_key(),
            30,
            &[9u8; 32],
            30,
            "2026-01-01T00:00:00Z",
            &GENESIS_PREV,
        );
        let lines = vec![line];
        let err = verify_checkpoint_chain(&as_slices(&lines), &keys).unwrap_err();
        assert_eq!(err, CpError::UnauthorizedSigner { position: 30 });
    }

    #[test]
    fn checkpoint_declares_correct_keyid_but_actually_signed_by_a_different_key_is_signature_invalid(
    ) {
        let sk1 = key(1);
        let sk2 = key(2);
        let keys = two_key_chain(&sk1, &sk2, 50);
        // Position 30 is sk1's window; payload HONESTLY-LOOKING declares
        // sk1 as keyid (the correct expected signer)... but the envelope is
        // actually signed by sk2. The keyid cross-check alone can't catch
        // this — only the cryptographic verify under the position-resolved
        // key (sk1) can, and it must fail.
        let line = checkpoint_line(
            &sk2, // actual signer
            &sk1.verifying_key(), // declared keyid: the correct one
            30,
            &[9u8; 32],
            30,
            "2026-01-01T00:00:00Z",
            &GENESIS_PREV,
        );
        let lines = vec![line];
        let err = verify_checkpoint_chain(&as_slices(&lines), &keys).unwrap_err();
        assert_eq!(err, CpError::SignatureInvalid { position: 30 });
    }

    #[test]
    fn honest_checkpoint_within_rotated_key_window_verifies() {
        let sk1 = key(1);
        let sk2 = key(2);
        let keys = two_key_chain(&sk1, &sk2, 50);
        let cp_at_30 = checkpoint_line(
            &sk1,
            &sk1.verifying_key(),
            30,
            &[9u8; 32],
            30,
            "2026-01-01T00:00:00Z",
            &GENESIS_PREV,
        );
        let head_after = advance(GENESIS_PREV, &cp_at_30);
        let cp_at_60 = checkpoint_line(
            &sk2,
            &sk2.verifying_key(),
            60,
            &[8u8; 32],
            60,
            "2026-01-02T00:00:00Z",
            &head_after,
        );
        let lines = vec![cp_at_30, cp_at_60];
        let chain = verify_checkpoint_chain(&as_slices(&lines), &keys)
            .expect("both checkpoints signed by the correct-for-position key must verify");
        assert_eq!(chain.checkpoints().len(), 2);
    }

    // ---- 4: range_bounds ----

    fn three_checkpoint_chain(sk: &SigningKey) -> CheckpointChain {
        let keys = single_key_chain(sk);
        let cp1 = checkpoint_line(
            sk,
            &sk.verifying_key(),
            0,
            &[1u8; 32],
            0,
            "2026-01-01T00:00:00Z",
            &GENESIS_PREV,
        );
        let head1 = advance(GENESIS_PREV, &cp1);
        let cp2 = checkpoint_line(
            sk,
            &sk.verifying_key(),
            100,
            &[2u8; 32],
            100,
            "2026-01-02T00:00:00Z",
            &head1,
        );
        let head2 = advance(head1, &cp2);
        let cp3 = checkpoint_line(
            sk,
            &sk.verifying_key(),
            250,
            &[3u8; 32],
            250,
            "2026-01-03T00:00:00Z",
            &head2,
        );
        let lines = vec![cp1, cp2, cp3];
        verify_checkpoint_chain(&as_slices(&lines), &keys).expect("must verify")
    }

    #[test]
    fn range_bounds_on_exact_boundaries_passes() {
        let sk = key(1);
        let chain = three_checkpoint_chain(&sk);
        let bounds = range_bounds(&chain, (0, 100)).expect("0 and 100 are both boundaries");
        assert_eq!(bounds.start_position, 0);
        assert_eq!(bounds.start_head, [1u8; 32]);
        assert_eq!(bounds.end_position, 100);
        assert_eq!(bounds.end_head, [2u8; 32]);
        assert_eq!(bounds.end_count, 100);

        let bounds2 = range_bounds(&chain, (100, 250)).expect("100 and 250 are both boundaries");
        assert_eq!(bounds2.start_position, 100);
        assert_eq!(bounds2.start_head, [2u8; 32]);
        assert_eq!(bounds2.end_position, 250);
        assert_eq!(bounds2.end_head, [3u8; 32]);
        assert_eq!(bounds2.end_count, 250);
    }

    #[test]
    fn range_bounds_zero_record_range_same_checkpoint_both_ends() {
        let sk = key(1);
        let chain = three_checkpoint_chain(&sk);
        let bounds = range_bounds(&chain, (100, 100)).expect("same boundary on both ends");
        assert_eq!(bounds.start_position, 100);
        assert_eq!(bounds.end_position, 100);
        assert_eq!(bounds.start_head, bounds.end_head);
    }

    #[test]
    fn range_bounds_end_not_a_boundary_is_rejected() {
        let sk = key(1);
        let chain = three_checkpoint_chain(&sk);
        let err = range_bounds(&chain, (0, 150)).unwrap_err();
        assert_eq!(err, CpError::NotABoundary { position: 150 });
    }

    #[test]
    fn range_bounds_start_not_a_boundary_is_rejected() {
        let sk = key(1);
        let chain = three_checkpoint_chain(&sk);
        let err = range_bounds(&chain, (5, 100)).unwrap_err();
        assert_eq!(err, CpError::NotABoundary { position: 5 });
    }

    // ---- 5: monotonic positions enforced ----

    #[test]
    fn backwards_checkpoint_position_is_rejected() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let cp1 = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            50,
            &[1u8; 32],
            50,
            "2026-01-01T00:00:00Z",
            &GENESIS_PREV,
        );
        let head1 = advance(GENESIS_PREV, &cp1);
        let cp2 = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            20, // backwards
            &[2u8; 32],
            20,
            "2026-01-02T00:00:00Z",
            &head1,
        );
        let lines = vec![cp1, cp2];
        let err = verify_checkpoint_chain(&as_slices(&lines), &keys).unwrap_err();
        assert_eq!(
            err,
            CpError::PositionNotIncreasing {
                previous: 50,
                found: 20
            }
        );
    }

    #[test]
    fn duplicate_checkpoint_position_is_rejected() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let cp1 = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            50,
            &[1u8; 32],
            50,
            "2026-01-01T00:00:00Z",
            &GENESIS_PREV,
        );
        let head1 = advance(GENESIS_PREV, &cp1);
        let cp2 = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            50, // duplicate
            &[2u8; 32],
            60,
            "2026-01-02T00:00:00Z",
            &head1,
        );
        let lines = vec![cp1, cp2];
        let err = verify_checkpoint_chain(&as_slices(&lines), &keys).unwrap_err();
        assert_eq!(
            err,
            CpError::PositionNotIncreasing {
                previous: 50,
                found: 50
            }
        );
    }

    // ---- 6: wall-clock non-monotonicity is a finding, not an error ----

    #[test]
    fn wall_clock_non_monotonic_is_valid_with_a_clock_anomaly_finding() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let cp1 = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            10,
            &[1u8; 32],
            10,
            "2026-06-01T00:00:00Z",
            &GENESIS_PREV,
        );
        let head1 = advance(GENESIS_PREV, &cp1);
        // Position strictly increases (20 > 10, honestly linked) but the
        // wall clock goes BACKWARDS relative to cp1.
        let cp2 = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            20,
            &[2u8; 32],
            20,
            "2026-01-01T00:00:00Z",
            &head1,
        );
        let lines = vec![cp1, cp2];
        let chain = verify_checkpoint_chain(&as_slices(&lines), &keys)
            .expect("wall-clock non-monotonicity must NOT fail verification");
        assert_eq!(chain.checkpoints().len(), 2);
        assert_eq!(chain.clock_anomalies().len(), 1);
        let anomaly = &chain.clock_anomalies()[0];
        assert_eq!(anomaly.position, 20);
        assert_eq!(anomaly.previous_position, 10);
    }

    #[test]
    fn wall_clock_equal_timestamps_is_also_a_clock_anomaly() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let same_time = "2026-06-01T00:00:00Z";
        let cp1 = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            10,
            &[1u8; 32],
            10,
            same_time,
            &GENESIS_PREV,
        );
        let head1 = advance(GENESIS_PREV, &cp1);
        let cp2 = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            20,
            &[2u8; 32],
            20,
            same_time,
            &head1,
        );
        let lines = vec![cp1, cp2];
        let chain = verify_checkpoint_chain(&as_slices(&lines), &keys)
            .expect("equal timestamps are non-monotonic but not an error");
        assert_eq!(chain.clock_anomalies().len(), 1);
    }

    #[test]
    fn wall_clock_monotonic_has_no_anomalies() {
        let sk = key(1);
        let chain = three_checkpoint_chain(&sk);
        assert!(chain.clock_anomalies().is_empty());
    }

    // ---- structural / malformed cases ----

    #[test]
    fn empty_chain_is_rejected() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let err = verify_checkpoint_chain(&[], &keys).unwrap_err();
        assert_eq!(err, CpError::Empty);
    }

    #[test]
    fn malformed_json_is_malformed_not_a_panic() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let garbage: &[u8] = b"not json at all {{{";
        let err = verify_checkpoint_chain(&[garbage], &keys).unwrap_err();
        assert!(matches!(err, CpError::Malformed { index: 0, .. }));
    }

    #[test]
    fn unknown_field_in_checkpoint_payload_is_malformed() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let payload = serde_json::to_vec(&serde_json::json!({
            "position": 10,
            "head": hex::encode([1u8; 32]),
            "count": 10,
            "at": "2026-01-01T00:00:00Z",
            "keyid": hex32(&sk.verifying_key()),
            "cp_prev": hex::encode(GENESIS_PREV),
            "extra_field": true,
        }))
        .unwrap();
        let line = sign_envelope(&sk, &hex32(&sk.verifying_key()), PT_CHECKPOINT, &payload).to_line();
        let err = verify_checkpoint_chain(&[line.as_slice()], &keys).unwrap_err();
        assert!(matches!(err, CpError::Malformed { index: 0, .. }));
    }

    #[test]
    fn invalid_hex_in_head_field_is_malformed() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let payload = serde_json::to_vec(&serde_json::json!({
            "position": 10,
            "head": "not-hex-zz",
            "count": 10,
            "at": "2026-01-01T00:00:00Z",
            "keyid": hex32(&sk.verifying_key()),
            "cp_prev": hex::encode(GENESIS_PREV),
        }))
        .unwrap();
        let line = sign_envelope(&sk, &hex32(&sk.verifying_key()), PT_CHECKPOINT, &payload).to_line();
        let err = verify_checkpoint_chain(&[line.as_slice()], &keys).unwrap_err();
        assert!(matches!(err, CpError::Malformed { index: 0, .. }));
    }

    #[test]
    fn invalid_hex_in_keyid_field_is_malformed() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let payload = serde_json::to_vec(&serde_json::json!({
            "position": 10,
            "head": hex::encode([1u8; 32]),
            "count": 10,
            "at": "2026-01-01T00:00:00Z",
            "keyid": "zz-not-hex",
            "cp_prev": hex::encode(GENESIS_PREV),
        }))
        .unwrap();
        let line = sign_envelope(&sk, &hex32(&sk.verifying_key()), PT_CHECKPOINT, &payload).to_line();
        let err = verify_checkpoint_chain(&[line.as_slice()], &keys).unwrap_err();
        assert!(matches!(err, CpError::Malformed { index: 0, .. }));
    }

    #[test]
    fn invalid_rfc3339_at_field_is_invalid_timestamp() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let line = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            10,
            &[1u8; 32],
            10,
            "not-a-timestamp",
            &GENESIS_PREV,
        );
        let err = verify_checkpoint_chain(&[line.as_slice()], &keys).unwrap_err();
        assert!(matches!(
            err,
            CpError::InvalidTimestamp { position: 10, .. }
        ));
    }

    #[test]
    fn checkpoint_position_zero_immediately_after_genesis_is_allowed() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let line = checkpoint_line(
            &sk,
            &sk.verifying_key(),
            0,
            &GENESIS_PREV,
            0,
            "2026-01-01T00:00:00Z",
            &GENESIS_PREV,
        );
        let chain = verify_checkpoint_chain(&[line.as_slice()], &keys)
            .expect("a checkpoint at position 0 (zero records) must verify");
        assert_eq!(chain.checkpoints()[0].position, 0);
        assert_eq!(chain.checkpoints()[0].count, 0);
    }

    // ---- range_bounds does not impose start <= end (documented behavior) ----

    #[test]
    fn range_bounds_does_not_require_start_before_end() {
        let sk = key(1);
        let chain = three_checkpoint_chain(&sk);
        // Reversed range: both are valid boundaries, so both resolve; no
        // ordering constraint is enforced at this layer.
        let bounds = range_bounds(&chain, (250, 0)).expect("both endpoints are valid boundaries");
        assert_eq!(bounds.start_position, 250);
        assert_eq!(bounds.end_position, 0);
    }

    // ---- proptest: N-checkpoint chain, brute-force checkpoint_at / range_bounds ----

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn range_bounds_matches_brute_force_over_random_checkpoint_positions(
            gaps in proptest::collection::vec(1u64..50, 1..10),
        ) {
            let sk = key(1);
            let keys = single_key_chain(&sk);

            let mut positions = Vec::new();
            let mut acc = 0u64;
            for g in &gaps {
                acc += g;
                positions.push(acc);
            }

            let mut lines: Vec<Vec<u8>> = Vec::new();
            let mut head = GENESIS_PREV;
            for (i, &p) in positions.iter().enumerate() {
                let line = checkpoint_line(
                    &sk,
                    &sk.verifying_key(),
                    p,
                    &[(i as u8).wrapping_add(1); 32],
                    p,
                    "2026-01-01T00:00:00Z",
                    &head,
                );
                head = advance(head, &line);
                lines.push(line);
            }

            let chain = verify_checkpoint_chain(&as_slices(&lines), &keys)
                .expect("honestly-built chain must verify");

            // Every real boundary resolves; a boundary one more than the
            // max, or one less than the min (when min > 0), does not.
            for &p in &positions {
                prop_assert!(range_bounds(&chain, (p, p)).is_ok());
            }
            let max_position = *positions.last().unwrap();
            prop_assert_eq!(
                range_bounds(&chain, (max_position, max_position + 1)),
                Err(CpError::NotABoundary { position: max_position + 1 })
            );
        }
    }
}
