// SPDX-License-Identifier: MIT
//! Full-link record-chain walker (spec law 4, taxonomy row 4).
//!
//! `walk_chain` is the "does verify actually verify?" primitive for chain
//! integrity: it recomputes the leaf/head hash for EVERY record in the
//! given slice and checks EVERY declared link, never just the endpoints
//! (the immudb CVE-2022-36111 lesson — an endpoint-only check false-passes
//! interior tampering). It also enforces strict position continuity: no
//! gaps, no duplicates, no reordering.
//!
//! Contract for callers (Task 7's verify pipeline, Task 9's writer):
//! `start_position`/`start_head` are a TRUSTED anchor — either the genesis
//! anchor `(0, GENESIS_PREV)` or a previously-verified checkpoint's declared
//! `(position, head)`. The first element of `envelope_lines` MUST declare
//! `position == start_position + 1` and `prev_head == start_head`; every
//! subsequent record continues from there. `walk_chain` does NOT verify
//! DSSE signatures (that is `verify_envelope`'s job, Task 2) — it verifies
//! only the hash-chain linkage and position sequence over the raw envelope
//! bytes. A caller that needs both runs both checks over the same lines.

use crate::hash::{head_hash, leaf_hash};
use crate::strict::from_slice_strict;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Deserialize;

/// Every variant is a fail-closed outcome: no case is ever silently
/// skipped or coerced into a successful walk.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChainError {
    /// `index` is the 0-based position into the `envelope_lines` slice (NOT
    /// the journal `position` field, which is unknown/untrusted for a line
    /// that failed to parse in the first place).
    #[error("record at slice index {index} is malformed: {reason}")]
    Malformed { index: usize, reason: String },

    /// The record at `position` declares a `prev_head` that does not match
    /// the head actually computed from everything before it.
    #[error("chain link mismatch at position {position}: declared prev_head does not match the computed chain head")]
    LinkMismatch { position: u64 },

    /// `expected` was the next continuous position; `found` is what the
    /// record actually declared (either ahead — a gap — or behind, but not
    /// an exact repeat of the last-consumed position).
    #[error("position gap: expected {expected}, found {found}")]
    PositionGap { expected: u64, found: u64 },

    /// The record declares the same `position` as the record immediately
    /// before it in the walk.
    #[error("duplicate position: {position}")]
    PositionDuplicate { position: u64 },

    /// The position counter would overflow `u64` on the next increment.
    /// Fails closed rather than wrapping (a silent wrap to 0 would let a
    /// forged chain masquerade as a fresh genesis).
    #[error("position counter overflow at slice index {index}")]
    PositionOverflow { index: usize },
}

/// Record payload schema (deserialized from the DSSE envelope's decoded
/// payload bytes — those bytes, not this struct, remain the hashed/signed
/// truth; `walk_chain` re-derives `leaf_hash` from the raw envelope line,
/// never from a re-serialization of this struct).
///
/// `deny_unknown_fields`: the schema is closed and fully specified by the
/// spec (design doc, record payload section); an unrecognized key is
/// treated as malformed input rather than silently ignored.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordHeader {
    pub position: u64,
    #[serde(deserialize_with = "deserialize_hex32")]
    pub prev_head: [u8; 32],
    pub class: String,
    pub kind: String,
    pub at: String,
    pub actor: serde_json::Value,
    #[serde(default, deserialize_with = "deserialize_hex32_opt")]
    pub content_hash: Option<[u8; 32]>,
    pub body: serde_json::Value,
    #[serde(default)]
    pub backfilled: bool,
}

fn hex_to_32(s: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(s).map_err(|e| format!("invalid hex: {e}"))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("expected 32 bytes, got {}", bytes.len()))
}

fn deserialize_hex32<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    hex_to_32(&s).map_err(serde::de::Error::custom)
}

fn deserialize_hex32_opt<'de, D>(deserializer: D) -> Result<Option<[u8; 32]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        None => Ok(None),
        Some(s) => hex_to_32(&s).map(Some).map_err(serde::de::Error::custom),
    }
}

/// Minimal, tolerant peek at the DSSE wire form to pull out just the
/// base64 `payload` field. This is deliberately NOT the strict, full
/// envelope parse/verify (that is `envelope::verify_envelope`, which also
/// checks `payloadType`, `sig`, `keyid`, and rejects the multi-signature
/// form). `walk_chain` only needs the payload bytes to read `position` and
/// `prev_head`; signature/keyid correctness is a separate, orthogonal
/// check the caller runs over the same raw lines.
#[derive(Deserialize)]
struct PayloadPeek {
    payload: String,
}

fn parse_record_header(line: &[u8], index: usize) -> Result<RecordHeader, ChainError> {
    let peek: PayloadPeek = from_slice_strict(line).map_err(|e| ChainError::Malformed {
        index,
        reason: format!("envelope JSON: {e}"),
    })?;
    let payload_bytes = STANDARD
        .decode(peek.payload.as_bytes())
        .map_err(|e| ChainError::Malformed {
            index,
            reason: format!("payload base64: {e}"),
        })?;
    from_slice_strict(&payload_bytes).map_err(|e| ChainError::Malformed {
        index,
        reason: format!("record payload JSON: {e}"),
    })
}

/// Public, additive read helper: parse ONE record envelope line into its
/// closed-schema [`RecordHeader`] (position, prev_head, class/kind, actor,
/// content_hash, body). A thin wrapper over the exact per-line parser
/// [`walk_chain`] uses internally — so offline tooling that needs record
/// bodies/positions (the CLI's `evidence why`/`tables`) reuses the trust
/// core's parser rather than growing a second one that could drift from it
/// (the parser-differential risk the design's Round-5 N4 warns against).
/// Does NO chain/signature verification of its own; callers that care about
/// authenticity gate on [`crate::verify::verify_bundle`] first.
pub fn parse_record(line: &[u8]) -> Result<RecordHeader, ChainError> {
    parse_record_header(line, 0)
}

/// Walk a slice of raw envelope lines starting from a trusted
/// `(start_position, start_head)`. Verifies EVERY link and strict position
/// continuity. Returns the final `(position, head)` reached, or the first
/// `ChainError` encountered (with the position/index of the failure).
///
/// An empty `envelope_lines` is a trivially valid walk: it returns
/// `(start_position, start_head)` unchanged (the zero-record range case).
pub fn walk_chain(
    start_position: u64,
    start_head: [u8; 32],
    envelope_lines: &[&[u8]],
) -> Result<(u64, [u8; 32]), ChainError> {
    // Single walk implementation, shared with `chain_heads`: the final
    // `(position, head)` is just the last running head the walk reached, or
    // the start anchor unchanged for an empty (zero-record) walk.
    let heads = chain_heads(start_position, start_head, envelope_lines)?;
    Ok(heads.last().copied().unwrap_or((start_position, start_head)))
}

/// Same strict walk as [`walk_chain`] (every link and position checked, no
/// gaps/dups/reorders, fail-closed on overflow), but returns the running
/// `(position, head)` AFTER every record instead of only the final pair.
/// `chain_heads[i]` is `(position_i, head_after_position_i)` for the i-th
/// record in `envelope_lines`; an empty slice yields an empty vec.
///
/// This is the per-position commitment the CLI's `--against` cross-bundle
/// consistency check compares: `head_after_P` is a collision-resistant
/// commitment to the ENTIRE chain history through position `P`, so two
/// bundles that agree on it at a shared position provably share all history
/// up to that position, and any disagreement at a shared position is
/// cryptographic proof of a fork. Comparing the running head (not the
/// record's declared `prev_head`, which only commits through `P-1`) is what
/// makes a fork whose only divergent record is the LAST shared position
/// still detectable — the difference that rules out a false "consistent".
/// [`walk_chain`] delegates here, so the two can never disagree.
pub fn chain_heads(
    start_position: u64,
    start_head: [u8; 32],
    envelope_lines: &[&[u8]],
) -> Result<Vec<(u64, [u8; 32])>, ChainError> {
    let mut position = start_position;
    let mut head = start_head;
    let mut heads = Vec::with_capacity(envelope_lines.len());

    for (index, line) in envelope_lines.iter().enumerate() {
        let record = parse_record_header(line, index)?;

        let expected_position = position
            .checked_add(1)
            .ok_or(ChainError::PositionOverflow { index })?;

        if record.position != expected_position {
            if record.position == position {
                return Err(ChainError::PositionDuplicate {
                    position: record.position,
                });
            }
            return Err(ChainError::PositionGap {
                expected: expected_position,
                found: record.position,
            });
        }

        if record.prev_head != head {
            return Err(ChainError::LinkMismatch {
                position: record.position,
            });
        }

        let leaf = leaf_hash(line);
        head = head_hash(&head, &leaf);
        position = record.position;
        heads.push((position, head));
    }

    Ok(heads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{sign_envelope, PT_RECORD};
    use crate::hash::GENESIS_PREV;
    use ed25519_dalek::SigningKey;
    use proptest::prelude::*;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    /// Builds one record's payload JSON bytes for the given `position`,
    /// `prev_head`, and a small per-record seed so `body` differs.
    fn record_payload(position: u64, prev_head: &[u8; 32], seed: u64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "position": position,
            "prev_head": hex::encode(prev_head),
            "class": "test",
            "kind": "note",
            "at": "2026-08-24T00:00:00Z",
            "actor": {"id": format!("actor-{seed}")},
            "content_hash": null,
            "body": {"seq": seed},
            "backfilled": false,
        }))
        .expect("static JSON value always serializes")
    }

    /// Builds a chain of `n` signed record envelope lines starting right
    /// after `(start_position, start_head)`, i.e. positions
    /// `start_position+1 ..= start_position+n`. Returns the lines and the
    /// honest final head.
    fn build_chain(
        sk: &SigningKey,
        start_position: u64,
        start_head: [u8; 32],
        n: u64,
    ) -> (Vec<Vec<u8>>, [u8; 32]) {
        let mut lines = Vec::new();
        let mut position = start_position;
        let mut head = start_head;
        for i in 0..n {
            let next_position = position + 1;
            let payload = record_payload(next_position, &head, i);
            let env = sign_envelope(sk, "test-key", PT_RECORD, &payload);
            let line = env.to_line();
            let leaf = leaf_hash(&line);
            head = head_hash(&head, &leaf);
            lines.push(line);
            position = next_position;
        }
        (lines, head)
    }

    fn as_slices(lines: &[Vec<u8>]) -> Vec<&[u8]> {
        lines.iter().map(|l| l.as_slice()).collect()
    }

    // ---- genesis / trivial walks ----

    #[test]
    fn genesis_walk_with_zero_records_returns_start_unchanged() {
        let got = walk_chain(0, GENESIS_PREV, &[]).expect("empty walk must succeed");
        assert_eq!(got, (0, GENESIS_PREV));
    }

    #[test]
    fn zero_record_walk_from_an_arbitrary_checkpoint_returns_start_unchanged() {
        let anchor_head = [42u8; 32];
        let got = walk_chain(77, anchor_head, &[]).expect("empty walk must succeed");
        assert_eq!(got, (77, anchor_head));
    }

    // ---- chain_heads: per-position running heads (the --against primitive) ----

    #[test]
    fn chain_heads_empty_walk_is_empty() {
        let heads = chain_heads(0, GENESIS_PREV, &[]).expect("empty walk must succeed");
        assert!(heads.is_empty());
    }

    #[test]
    fn chain_heads_records_the_head_after_every_position_and_agrees_with_walk_chain() {
        let sk = key(1);
        let (lines, honest_head) = build_chain(&sk, 0, GENESIS_PREV, 10);
        let heads = chain_heads(0, GENESIS_PREV, &as_slices(&lines)).expect("must walk");
        // One entry per record, positions 1..=10 in order.
        assert_eq!(heads.len(), 10);
        for (i, (pos, _)) in heads.iter().enumerate() {
            assert_eq!(*pos, i as u64 + 1);
        }
        // The LAST running head is exactly what walk_chain reports as final.
        assert_eq!(heads.last().copied(), Some((10, honest_head)));
        assert_eq!(
            walk_chain(0, GENESIS_PREV, &as_slices(&lines)).unwrap(),
            (10, honest_head)
        );
    }

    #[test]
    fn chain_heads_diverge_at_the_first_differing_record() {
        // Two honest 5-record chains that share records 1..3 byte-for-byte
        // (same seeds) but differ from record 4 on: the running head must be
        // identical through position 3 and differ from position 4 onward —
        // the exact property --against relies on to localize a fork.
        let sk = key(1);
        let mut a_lines = Vec::new();
        let mut b_lines = Vec::new();
        let mut a_head = GENESIS_PREV;
        let mut b_head = GENESIS_PREV;
        for i in 0..5u64 {
            let pos = i + 1;
            // Records 1..3 identical; 4..5 use a different body seed in B.
            let a_payload = record_payload(pos, &a_head, i);
            let b_seed = if pos <= 3 { i } else { i + 1000 };
            let b_payload = record_payload(pos, &b_head, b_seed);
            let a_env = sign_envelope(&sk, "k", PT_RECORD, &a_payload);
            let b_env = sign_envelope(&sk, "k", PT_RECORD, &b_payload);
            let a_line = a_env.to_line();
            let b_line = b_env.to_line();
            a_head = head_hash(&a_head, &leaf_hash(&a_line));
            b_head = head_hash(&b_head, &leaf_hash(&b_line));
            a_lines.push(a_line);
            b_lines.push(b_line);
        }
        let a = chain_heads(0, GENESIS_PREV, &as_slices(&a_lines)).unwrap();
        let b = chain_heads(0, GENESIS_PREV, &as_slices(&b_lines)).unwrap();
        assert_eq!(a[0], b[0]); // position 1 head equal
        assert_eq!(a[2], b[2]); // position 3 head equal
        assert_ne!(a[3].1, b[3].1); // position 4 head differs
        assert_ne!(a[4].1, b[4].1); // position 5 head differs
    }

    // ---- parse_record: public thin wrapper over the internal parser ----

    #[test]
    fn parse_record_reads_the_closed_schema_payload() {
        let sk = key(1);
        let payload = record_payload(1, &GENESIS_PREV, 42);
        let env = sign_envelope(&sk, "k", PT_RECORD, &payload);
        let line = env.to_line();
        let header = parse_record(&line).expect("well-formed record must parse");
        assert_eq!(header.position, 1);
        assert_eq!(header.prev_head, GENESIS_PREV);
        assert_eq!(header.kind, "note");
        assert_eq!(header.body, serde_json::json!({"seq": 42}));
    }

    #[test]
    fn parse_record_rejects_garbage() {
        let err = parse_record(b"not json at all {{{").unwrap_err();
        assert!(matches!(err, ChainError::Malformed { .. }));
    }

    // ---- single record ----

    #[test]
    fn single_record_chain_from_genesis() {
        let sk = key(1);
        let (lines, honest_head) = build_chain(&sk, 0, GENESIS_PREV, 1);
        let got = walk_chain(0, GENESIS_PREV, &as_slices(&lines)).expect("must verify");
        assert_eq!(got, (1, honest_head));
    }

    #[test]
    fn single_record_chain_resuming_from_an_arbitrary_checkpoint() {
        let sk = key(1);
        let start_head = [9u8; 32];
        let (lines, honest_head) = build_chain(&sk, 41, start_head, 1);
        let got = walk_chain(41, start_head, &as_slices(&lines)).expect("must verify");
        assert_eq!(got, (42, honest_head));
    }

    // ---- 100-record chain ----

    #[test]
    fn hundred_record_chain_verifies_and_lands_on_the_honest_head() {
        let sk = key(1);
        let (lines, honest_head) = build_chain(&sk, 0, GENESIS_PREV, 100);
        let got = walk_chain(0, GENESIS_PREV, &as_slices(&lines)).expect("must verify");
        assert_eq!(got, (100, honest_head));
    }

    // ---- flipped byte in a record's own prev_head field ----

    #[test]
    fn flipping_a_byte_in_record_50s_own_prev_head_is_caught_at_position_50() {
        let sk = key(1);
        let (mut lines, _honest_head) = build_chain(&sk, 0, GENESIS_PREV, 100);

        // Rebuild record at position 50 (index 49) with ONE bit flipped in
        // its declared prev_head hex string, then re-sign it (a forger with
        // the key would do exactly this; without the key the envelope
        // wouldn't even verify — but walk_chain doesn't check signatures,
        // so this isolates the link-check specifically).
        let tampered_index = 49usize; // position 50
        let (earlier_lines, head_before_50) = build_chain(&sk, 0, GENESIS_PREV, 49);
        assert_eq!(earlier_lines.len(), 49);
        let mut bad_prev = head_before_50;
        bad_prev[0] ^= 0x01; // flip one bit
        let bad_payload = record_payload(50, &bad_prev, 49);
        let bad_env = sign_envelope(&sk, "test-key", PT_RECORD, &bad_payload);
        lines[tampered_index] = bad_env.to_line();

        let err = walk_chain(0, GENESIS_PREV, &as_slices(&lines)).unwrap_err();
        assert_eq!(err, ChainError::LinkMismatch { position: 50 });
    }

    // ---- duplicate position ----

    #[test]
    fn duplicate_position_is_rejected() {
        let sk = key(1);
        let (mut lines, _) = build_chain(&sk, 0, GENESIS_PREV, 3);
        // Duplicate record at position 2 (index 1) right after itself,
        // instead of the real record at position 3.
        lines[2] = lines[1].clone();
        let err = walk_chain(0, GENESIS_PREV, &as_slices(&lines)).unwrap_err();
        assert_eq!(err, ChainError::PositionDuplicate { position: 2 });
    }

    // ---- gap ----

    #[test]
    fn position_gap_is_rejected() {
        let sk = key(1);
        let (lines, _) = build_chain(&sk, 0, GENESIS_PREV, 5);
        // Drop record at index 2 (position 3) entirely, leaving a gap
        // between position 2 and position 4.
        let mut with_gap = lines.clone();
        with_gap.remove(2);
        let err = walk_chain(0, GENESIS_PREV, &as_slices(&with_gap)).unwrap_err();
        assert_eq!(
            err,
            ChainError::PositionGap {
                expected: 3,
                found: 4
            }
        );
    }

    // ---- reorder ----

    #[test]
    fn reordering_two_adjacent_records_fails() {
        let sk = key(1);
        let (mut lines, _) = build_chain(&sk, 0, GENESIS_PREV, 5);
        lines.swap(2, 3); // positions 3 and 4 swap places
        let err = walk_chain(0, GENESIS_PREV, &as_slices(&lines)).unwrap_err();
        // Whatever the exact classification, a reorder must not verify.
        assert!(matches!(
            err,
            ChainError::PositionGap { .. } | ChainError::LinkMismatch { .. }
        ));
    }

    // ---- malformed input ----

    #[test]
    fn garbage_bytes_are_malformed_not_a_panic() {
        let garbage: &[u8] = b"not json at all {{{";
        let err = walk_chain(0, GENESIS_PREV, &[garbage]).unwrap_err();
        assert!(matches!(err, ChainError::Malformed { index: 0, .. }));
    }

    #[test]
    fn invalid_payload_base64_is_malformed() {
        let line = br#"{"payloadType":"t","payload":"not-valid-base64!!","sig":"","keyid":""}"#;
        let err = walk_chain(0, GENESIS_PREV, &[line.as_slice()]).unwrap_err();
        assert!(matches!(err, ChainError::Malformed { index: 0, .. }));
    }

    #[test]
    fn prev_head_wrong_hex_length_is_malformed() {
        let sk = key(1);
        let payload = serde_json::to_vec(&serde_json::json!({
            "position": 1,
            "prev_head": "ab", // way too short
            "class": "test",
            "kind": "note",
            "at": "2026-08-24T00:00:00Z",
            "actor": {},
            "content_hash": null,
            "body": {},
            "backfilled": false,
        }))
        .unwrap();
        let env = sign_envelope(&sk, "k", PT_RECORD, &payload);
        let line = env.to_line();
        let err = walk_chain(0, GENESIS_PREV, &[line.as_slice()]).unwrap_err();
        assert!(matches!(err, ChainError::Malformed { index: 0, .. }));
    }

    #[test]
    fn unknown_field_in_record_payload_is_malformed() {
        let sk = key(1);
        let payload = serde_json::to_vec(&serde_json::json!({
            "position": 1,
            "prev_head": hex::encode(GENESIS_PREV),
            "class": "test",
            "kind": "note",
            "at": "2026-08-24T00:00:00Z",
            "actor": {},
            "content_hash": null,
            "body": {},
            "backfilled": false,
            "extra_unrecognized_field": true,
        }))
        .unwrap();
        let env = sign_envelope(&sk, "k", PT_RECORD, &payload);
        let line = env.to_line();
        let err = walk_chain(0, GENESIS_PREV, &[line.as_slice()]).unwrap_err();
        assert!(matches!(err, ChainError::Malformed { index: 0, .. }));
    }

    // ---- position counter overflow ----

    #[test]
    fn position_overflow_fails_closed_instead_of_wrapping() {
        let sk = key(1);
        let payload = record_payload(0, &GENESIS_PREV, 0); // position value is irrelevant; overflow triggers before the comparison
        let env = sign_envelope(&sk, "k", PT_RECORD, &payload);
        let line = env.to_line();
        let err = walk_chain(u64::MAX, GENESIS_PREV, &[line.as_slice()]).unwrap_err();
        assert_eq!(err, ChainError::PositionOverflow { index: 0 });
    }

    // ---- the immudb lesson (REQUIRED case 2.10) ----

    /// Build a 200-record chain. Tamper ONLY the `body` content of the
    /// record at position 100 (not its `position` or `prev_head` fields,
    /// and nothing about records 0/genesis or 199 — the endpoints). Prove
    /// `walk_chain` still catches it, unlike a checker that validates only
    /// the endpoints (the immudb CVE-2022-36111 pattern).
    ///
    /// Mechanics, spelled out because the exact failure position is worth
    /// being precise about: `leaf_hash` covers the ENTIRE envelope line, so
    /// corrupting record 100's body changes its leaf and therefore the head
    /// computed after it. Record 100's OWN link check still passes (its
    /// declared `prev_head` — built from records 1..99, untouched — still
    /// matches). The corruption is caught one step later: record 101's
    /// declared `prev_head` (built from the ORIGINAL, honest head-at-100)
    /// no longer matches the head `walk_chain` just computed from the
    /// tampered record 100. So the reported failure position is 101, the
    /// first point where the chain can observe the divergence — not 100
    /// itself, since nothing else in the bundle independently attests to
    /// what record 100's head "should" have been. This is exactly why the
    /// check must walk every position: a checker that jumped straight to
    /// position 199 (the closing endpoint) would see a record whose OWN
    /// fields look perfectly self-consistent in isolation and would never
    /// learn that anything upstream was wrong.
    #[test]
    fn tampering_an_interior_record_is_caught_even_though_both_endpoints_are_untouched() {
        let sk = key(1);
        let (mut lines, honest_final_head) = build_chain(&sk, 0, GENESIS_PREV, 200);

        // Sanity: capture the untouched endpoints before tampering, to
        // demonstrate they remain individually well-formed afterward.
        let genesis_line = lines[0].clone();
        let last_line = lines[199].clone();

        // Tamper record 100 (index 99): re-sign the SAME position/prev_head
        // with different `body` content. Records 101..199 are left byte-
        // for-byte as originally built — they still declare the ORIGINAL
        // (honest) prev_head for position 100's successor.
        let tampered_index = 99usize;
        let (earlier_lines, head_before_100) = build_chain(&sk, 0, GENESIS_PREV, 99);
        assert_eq!(earlier_lines.len(), 99);
        let tampered_payload = record_payload(100, &head_before_100, 9_999_999);
        let tampered_env = sign_envelope(&sk, "test-key", PT_RECORD, &tampered_payload);
        lines[tampered_index] = tampered_env.to_line();

        // Endpoints are untouched by the tamper.
        assert_eq!(lines[0], genesis_line);
        assert_eq!(lines[199], last_line);

        let err = walk_chain(0, GENESIS_PREV, &as_slices(&lines)).unwrap_err();
        assert_eq!(err, ChainError::LinkMismatch { position: 101 });

        // A naive endpoint-only checker would inspect only records 0 and
        // 199 in isolation: both parse fine, both are individually
        // self-consistent (record 0's prev_head is GENESIS_PREV; record
        // 199 is a validly-signed envelope with a well-formed payload) —
        // an endpoint-only check would see nothing wrong here. Only the
        // full walk (proven above) catches the interior tamper.
        let genesis_record = parse_record_header(&lines[0], 0).expect("genesis parses");
        assert_eq!(genesis_record.prev_head, GENESIS_PREV);
        let last_record = parse_record_header(&lines[199], 199).expect("last record parses");
        assert_eq!(last_record.position, 200);

        // And the full walk did NOT silently land on the honest final head
        // despite the interior tamper (the false-VALID this whole test
        // exists to rule out).
        assert_ne!(
            walk_chain(0, GENESIS_PREV, &as_slices(&lines[..199])),
            Ok((199, honest_final_head))
        );
    }

    // ---- proptest: any single-byte flip in any (non-terminal) record fails ----

    // The mutation property in miniature (task 16 runs the exhaustive
    // version). A chain of `n` (1..40) honestly-built records is followed
    // by ONE extra sentinel record. We flip a single byte in one of the
    // first `n` records (never the sentinel) and assert the walk fails.
    //
    // Why the sentinel is required, not decorative: `leaf_hash` covers the
    // whole envelope line, so a byte flip confined to a record's `body`
    // changes that record's leaf/head but doesn't, by itself, violate any
    // check `walk_chain` can perform ON THAT RECORD alone (its own
    // `position`/`prev_head` fields are untouched). The corruption is only
    // observable at the NEXT record's link check. Without a guaranteed
    // "next" record, a flip landing in the trailing record's `body` would
    // make `walk_chain` return `Ok` with a merely-different head — not an
    // `Err` — which would make the literal "always fails" property false.
    // The sentinel guarantees a next record always exists for every
    // mutated position, closing that gap.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn any_single_byte_flip_in_any_record_breaks_the_walk(
            n in 1u64..40,
            flip_record_idx in 0usize..39,
            flip_byte_idx in 0usize..2000,
        ) {
            let sk = key(3);
            // n honest records + 1 sentinel = n+1 total.
            let (mut lines, _honest_head) = build_chain(&sk, 0, GENESIS_PREV, n + 1);
            let flip_record_idx = flip_record_idx % (n as usize); // never the sentinel (index n)
            let line = &mut lines[flip_record_idx];
            if line.is_empty() {
                return Ok(());
            }
            let flip_byte_idx = flip_byte_idx % line.len();
            line[flip_byte_idx] ^= 0x01;

            let result = walk_chain(0, GENESIS_PREV, &as_slices(&lines));
            prop_assert!(result.is_err());
        }
    }
}
