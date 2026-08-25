// SPDX-License-Identifier: MIT
//! Key-record chain: genesis + rotation + compromise records, position-based
//! key validity, and compromise classification (spec taxonomy rows 3, 5, 6,
//! 7, 8, 9).
//!
//! Key records form their OWN hash chain (`key_prev`), separate from the
//! evidence record chain (`chain::walk_chain`) and the checkpoint chain, but
//! sharing the SAME position coordinate space: a rotation/compromise
//! record's `position` field names the evidence-record position from which
//! it takes effect. Positions here need not be contiguous (rotations are
//! sparse) but MUST strictly increase, and `key_prev` is a
//! `leaf_hash`/`head_hash` chain over the key-record envelopes exactly
//! analogous to `walk_chain`'s tamper-evidence for the record chain (design
//! doc "Key records").
//!
//! Position convention (controller ruling): genesis is a REAL, self-signed
//! envelope at the literal position `0` — the key it declares is valid from
//! the very start, covering the virtual `(0, GENESIS_PREV)` anchor and every
//! real evidence record from position 1 onward. Key validity is a half-open
//! interval: a key declared at position `D` is valid for `[D, successor_D)`
//! — the declaration position belongs to the NEW key, pinned by a boundary
//! test below.
//!
//! Compromise semantics (Round-5-hardened design): a compromise record at
//! position `P`, signed by the genesis-declared recovery key, names a
//! compromised signing key `K` and carries a `claimed_compromise_time` `C`.
//! It SEALS the chain — no further key record may follow it
//! (`KeyChainError::JournalSealed`; evidence-record sealing past `P` is
//! `classify_compromise`'s `TamperedPostPosition`, enforced by Task 7's
//! pipeline). Classifying whether a given evidence record signed by `K` is
//! trustworthy is `classify_compromise`'s job; it is PURE — no anchor
//! validation, no wall clock, inside this function. The caller establishes
//! `anchored_before_claim` from the checkpoint/anchor chain (a checkpoint
//! covering the record, anchored at tier >= 2, with a VALID anchor time
//! strictly before `C`) and passes it in.

use crate::envelope::{verify_envelope, EnvelopeError, PT_KEYRECORD};
use crate::hash::{head_hash, leaf_hash, GENESIS_PREV};
use crate::strict::from_slice_strict;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use ed25519_dalek::VerifyingKey;
use serde::Deserialize;

/// Every variant is a fail-closed outcome: no case is silently skipped or
/// coerced into a successful/partial chain.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyChainError {
    #[error("key record at slice index {index} is malformed: {reason}")]
    Malformed { index: usize, reason: String },

    #[error("key chain must begin with a genesis record (kind=\"genesis\", position=0)")]
    MissingGenesis,

    #[error("genesis record declared position {found}, must be 0")]
    GenesisPositionMismatch { found: u64 },

    #[error("a second genesis record appears at slice index {index}")]
    DuplicateGenesis { index: usize },

    #[error("genesis key record is not self-signed by its declared signing_key")]
    GenesisNotSelfSigned,

    #[error("unknown key-record kind {kind:?} at slice index {index}")]
    UnknownKind { index: usize, kind: String },

    #[error("key-chain position must strictly increase: previous {previous}, found {found}")]
    PositionNotIncreasing { previous: u64, found: u64 },

    #[error("key_prev at position {position} does not match the computed key-chain head")]
    KeyLinkMismatch { position: u64 },

    #[error(
        "rotation record at position {position} is not signed by the predecessor signing key"
    )]
    UnauthorizedRotation { position: u64 },

    #[error(
        "compromise record at position {position} is not signed by the declared recovery key (or none was declared)"
    )]
    UnauthorizedControlRecord { position: u64 },

    #[error(
        "compromise record at position {position} has an invalid claimed_compromise_time (must be RFC 3339): {reason}"
    )]
    InvalidCompromiseTime { position: u64, reason: String },

    #[error(
        "key record at slice index {index} appears after the journal was sealed by the compromise at position {sealed_at}"
    )]
    JournalSealed { index: usize, sealed_at: u64 },
}

fn hex_to_32(s: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(s).map_err(|e| format!("invalid hex: {e}"))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("expected 32 bytes, got {}", bytes.len()))
}

fn parse_verifying_key(
    hex_str: &str,
    index: usize,
    field: &str,
) -> Result<VerifyingKey, KeyChainError> {
    let bytes = hex_to_32(hex_str).map_err(|reason| KeyChainError::Malformed {
        index,
        reason: format!("{field}: {reason}"),
    })?;
    VerifyingKey::from_bytes(&bytes).map_err(|e| KeyChainError::Malformed {
        index,
        reason: format!("{field}: invalid ed25519 public key: {e}"),
    })
}

/// Minimal, tolerant peek at the DSSE wire form to pull out just the base64
/// `payload` field — mirrors `chain::parse_record_header`'s split between a
/// tolerant structural peek and the strict, closed-schema parse below.
#[derive(Deserialize)]
struct PayloadPeek {
    payload: String,
}

fn decode_payload_bytes(line: &[u8], index: usize) -> Result<Vec<u8>, KeyChainError> {
    let peek: PayloadPeek =
        from_slice_strict(line).map_err(|e| KeyChainError::Malformed {
            index,
            reason: format!("envelope JSON: {e}"),
        })?;
    STANDARD
        .decode(peek.payload.as_bytes())
        .map_err(|e| KeyChainError::Malformed {
            index,
            reason: format!("payload base64: {e}"),
        })
}

/// Tolerant peek at just the `kind` discriminator, before committing to one
/// of the three closed-schema payload structs below.
#[derive(Deserialize)]
struct KindPeek {
    kind: String,
}

fn peek_kind(payload_bytes: &[u8], index: usize) -> Result<String, KeyChainError> {
    let peek: KindPeek =
        from_slice_strict(payload_bytes).map_err(|e| KeyChainError::Malformed {
            index,
            reason: format!("payload JSON (kind peek): {e}"),
        })?;
    Ok(peek.kind)
}

fn decode_json<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
    index: usize,
    what: &str,
) -> Result<T, KeyChainError> {
    from_slice_strict(bytes).map_err(|e| KeyChainError::Malformed {
        index,
        reason: format!("{what} payload JSON: {e}"),
    })
}

/// `deny_unknown_fields`: the schema is closed (design doc + Task 4 brief);
/// an unrecognized key is malformed input, not silently ignored. `_kind` is
/// deserialized (it IS present on the wire, and `deny_unknown_fields` would
/// reject it as unknown otherwise) but never re-read — the dispatch already
/// happened via `peek_kind` against the same bytes.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GenesisPayload {
    #[serde(rename = "kind")]
    _kind: String,
    position: u64,
    signing_key: String,
    recovery_key: Option<String>,
    // Present on the wire (successor-genesis lineage per the design doc's
    // "post-compromise stance" section) but not interpreted by this task:
    // cross-genesis continuity is attested, not proven, and rendering that
    // attestation is the verifier's (Task 7's) job, not the key chain's.
    #[serde(rename = "predecessor_genesis")]
    _predecessor_genesis: Option<serde_json::Value>,
    key_prev: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RotationPayload {
    #[serde(rename = "kind")]
    _kind: String,
    position: u64,
    new_signing_key: String,
    key_prev: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompromisePayload {
    #[serde(rename = "kind")]
    _kind: String,
    position: u64,
    compromised_key: String,
    claimed_compromise_time: String,
    key_prev: String,
}

/// A signing key becoming authoritative at `position` (half-open interval;
/// see module docs).
#[derive(Debug, Clone, Copy)]
struct SigningKeyEvent {
    position: u64,
    verifying_key: VerifyingKey,
}

/// The compromise record that sealed a [`KeyChain`], if any (spec rows 6-9).
/// `claimed_compromise_time` is validated (RFC 3339) at the `verify_key_chain`
/// boundary, not a raw wire string — Task 7 compares it against anchor times
/// to establish `anchored_before_claim`, so it must already be a trustworthy
/// timestamp by the time any caller can observe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompromiseRecord {
    pub position: u64,
    pub compromised_key: [u8; 32],
    pub claimed_compromise_time: DateTime<Utc>,
}

/// A verified key sub-chain: genesis plus every rotation/compromise record,
/// linked by `key_prev` and authorized per record kind. Only constructible
/// via [`verify_key_chain`] — its existence IS the proof that every linkage
/// and signature check already passed (parse-at-the-boundary, trust
/// inside).
#[derive(Debug, Clone)]
pub struct KeyChain {
    /// Ascending by position; `events[0]` is always genesis at position 0.
    events: Vec<SigningKeyEvent>,
    recovery_key: Option<VerifyingKey>,
    compromise: Option<CompromiseRecord>,
}

impl KeyChain {
    /// The genesis-declared cold recovery key, if any.
    pub fn recovery_key(&self) -> Option<&VerifyingKey> {
        self.recovery_key.as_ref()
    }

    /// The compromise record that sealed this chain, if any.
    pub fn compromise(&self) -> Option<&CompromiseRecord> {
        self.compromise.as_ref()
    }

    /// Every distinct signing key ever declared in this chain — genesis plus
    /// every rotation's `new_signing_key` — in position order. Added for
    /// Task 7 (the verdict engine): distinguishing "signature invalid under
    /// any key this chain has ever declared" (taxonomy row 2) from
    /// "signature valid, but under a key that was authoritative at a
    /// DIFFERENT position" (row 3, key-epoch violation) requires trying a
    /// record's signature against every historical key, not just the one
    /// [`key_at_position`] resolves for its declared position.
    pub fn all_signing_keys(&self) -> impl Iterator<Item = &VerifyingKey> {
        self.events.iter().map(|event| &event.verifying_key)
    }
}

/// Verify the full key sub-chain from genesis: `key_prev` linkage, correct
/// signers per record kind, and `payloadType`. Fails on: rotation not
/// signed by predecessor; compromise not signed by declared recovery key;
/// compromise when no recovery key declared; any key record after a
/// compromise (journal sealed). See module docs for the exact schema.
pub fn verify_key_chain(lines: &[&[u8]]) -> Result<KeyChain, KeyChainError> {
    let Some(genesis_line) = lines.first() else {
        return Err(KeyChainError::MissingGenesis);
    };

    let genesis_payload_bytes = decode_payload_bytes(genesis_line, 0)?;
    let kind = peek_kind(&genesis_payload_bytes, 0)?;
    if kind != "genesis" {
        return Err(KeyChainError::MissingGenesis);
    }
    let genesis: GenesisPayload = decode_json(&genesis_payload_bytes, 0, "genesis")?;
    if genesis.position != 0 {
        return Err(KeyChainError::GenesisPositionMismatch {
            found: genesis.position,
        });
    }
    let genesis_key_prev = hex_to_32(&genesis.key_prev).map_err(|reason| KeyChainError::Malformed {
        index: 0,
        reason: format!("key_prev: {reason}"),
    })?;
    if genesis_key_prev != GENESIS_PREV {
        return Err(KeyChainError::KeyLinkMismatch { position: 0 });
    }
    let signing_vk = parse_verifying_key(&genesis.signing_key, 0, "signing_key")?;
    // Genesis is the trust root: it must be self-signed by the very key it
    // declares (TOFU, same structure as a self-signed root certificate).
    // Only an actual cryptographic authenticity failure (SignatureInvalid)
    // is "not self-signed"; a wrong-payloadType/malformed/unsupported
    // envelope is a DIFFERENT failure class (Task 7 controller ruling,
    // 2026-08-24 review) — collapsing them would make a spliced
    // wrong-payloadType envelope read as GenesisNotSelfSigned, which the
    // verdict engine maps to a TAMPERED-flavored row, when the taxonomy
    // wants CANNOT_VERIFY(malformed/unsupported) instead.
    verify_envelope(genesis_line, PT_KEYRECORD, &signing_vk).map_err(|e| match e {
        EnvelopeError::SignatureInvalid => KeyChainError::GenesisNotSelfSigned,
        other => KeyChainError::Malformed {
            index: 0,
            reason: format!("genesis envelope: {other}"),
        },
    })?;
    let recovery_vk = match &genesis.recovery_key {
        None => None,
        Some(hex_key) => Some(parse_verifying_key(hex_key, 0, "recovery_key")?),
    };

    let mut events = vec![SigningKeyEvent {
        position: 0,
        verifying_key: signing_vk,
    }];
    let mut current_signing_vk = signing_vk;
    let mut compromise: Option<CompromiseRecord> = None;
    let mut last_position = 0u64;
    let mut head = head_hash(&GENESIS_PREV, &leaf_hash(genesis_line));

    for (index, line) in lines.iter().enumerate().skip(1) {
        // The journal is sealed the moment a compromise record is
        // consumed: reject ANYTHING further, without even attempting to
        // parse it — sealing is absolute, not conditional on content.
        if let Some(sealed) = &compromise {
            return Err(KeyChainError::JournalSealed {
                index,
                sealed_at: sealed.position,
            });
        }

        let payload_bytes = decode_payload_bytes(line, index)?;
        let kind = peek_kind(&payload_bytes, index)?;

        match kind.as_str() {
            "genesis" => return Err(KeyChainError::DuplicateGenesis { index }),
            "rotation" => {
                let rotation: RotationPayload = decode_json(&payload_bytes, index, "rotation")?;
                if rotation.position <= last_position {
                    return Err(KeyChainError::PositionNotIncreasing {
                        previous: last_position,
                        found: rotation.position,
                    });
                }
                let declared_prev =
                    hex_to_32(&rotation.key_prev).map_err(|reason| KeyChainError::Malformed {
                        index,
                        reason: format!("key_prev: {reason}"),
                    })?;
                if declared_prev != head {
                    return Err(KeyChainError::KeyLinkMismatch {
                        position: rotation.position,
                    });
                }
                // Rotation must be signed by the PREDECESSOR signing key —
                // the key authoritative right before this rotation. Same
                // un-collapse as genesis above: only SignatureInvalid means
                // "not authorized"; a wrong-payloadType/malformed/
                // unsupported envelope is CANNOT_VERIFY-flavored, not
                // TAMPERED-flavored.
                verify_envelope(line, PT_KEYRECORD, &current_signing_vk).map_err(|e| match e {
                    EnvelopeError::SignatureInvalid => KeyChainError::UnauthorizedRotation {
                        position: rotation.position,
                    },
                    other => KeyChainError::Malformed {
                        index,
                        reason: format!("rotation envelope: {other}"),
                    },
                })?;
                let new_vk =
                    parse_verifying_key(&rotation.new_signing_key, index, "new_signing_key")?;

                events.push(SigningKeyEvent {
                    position: rotation.position,
                    verifying_key: new_vk,
                });
                current_signing_vk = new_vk;
                last_position = rotation.position;
                head = head_hash(&head, &leaf_hash(line));
            }
            "compromise" => {
                let comp: CompromisePayload = decode_json(&payload_bytes, index, "compromise")?;
                if comp.position <= last_position {
                    return Err(KeyChainError::PositionNotIncreasing {
                        previous: last_position,
                        found: comp.position,
                    });
                }
                let declared_prev =
                    hex_to_32(&comp.key_prev).map_err(|reason| KeyChainError::Malformed {
                        index,
                        reason: format!("key_prev: {reason}"),
                    })?;
                if declared_prev != head {
                    return Err(KeyChainError::KeyLinkMismatch {
                        position: comp.position,
                    });
                }
                // Compromise must be signed by the genesis-declared
                // recovery key; no recovery key means no in-band authority
                // can ever produce a valid compromise record.
                let Some(rvk) = recovery_vk else {
                    return Err(KeyChainError::UnauthorizedControlRecord {
                        position: comp.position,
                    });
                };
                // Same un-collapse as genesis/rotation above.
                verify_envelope(line, PT_KEYRECORD, &rvk).map_err(|e| match e {
                    EnvelopeError::SignatureInvalid => KeyChainError::UnauthorizedControlRecord {
                        position: comp.position,
                    },
                    other => KeyChainError::Malformed {
                        index,
                        reason: format!("compromise envelope: {other}"),
                    },
                })?;
                let compromised_key =
                    hex_to_32(&comp.compromised_key).map_err(|reason| KeyChainError::Malformed {
                        index,
                        reason: format!("compromised_key: {reason}"),
                    })?;
                // Parse-at-the-boundary: `claimed_compromise_time` is a raw
                // wire string until here. Task 7 compares it against anchor
                // times, so a non-RFC3339 value must fail closed now rather
                // than silently propagate as an unvalidated string.
                let claimed_compromise_time =
                    DateTime::parse_from_rfc3339(&comp.claimed_compromise_time)
                        .map(|dt| dt.with_timezone(&Utc))
                        .map_err(|e| KeyChainError::InvalidCompromiseTime {
                            position: comp.position,
                            reason: e.to_string(),
                        })?;

                last_position = comp.position;
                head = head_hash(&head, &leaf_hash(line));
                // A compromise record does NOT introduce a new signing key
                // event — it seals the chain, it doesn't rotate it.
                compromise = Some(CompromiseRecord {
                    position: comp.position,
                    compromised_key,
                    claimed_compromise_time,
                });
            }
            other => {
                return Err(KeyChainError::UnknownKind {
                    index,
                    kind: other.to_string(),
                });
            }
        }
    }

    Ok(KeyChain {
        events,
        recovery_key: recovery_vk,
        compromise,
    })
}

/// Which signing key is authoritative at a given record position, per the
/// half-open interval rule: a key declared at position `D` is valid for
/// `[D, successor_D)`. The declaration position belongs to the NEW key:
/// this returns the event with the LARGEST declared position that is still
/// `<= position` (events are stored in strictly increasing position order,
/// enforced by `verify_key_chain`, so the first match scanning from the end
/// is exactly that event).
pub fn key_at_position(chain: &KeyChain, position: u64) -> Option<&VerifyingKey> {
    chain
        .events
        .iter()
        .rev()
        .find(|event| event.position <= position)
        .map(|event| &event.verifying_key)
}

/// Compromise classification (spec rows 7-9). Pure: `anchored_before_claim`
/// is the caller-established fact "this record is covered by a checkpoint
/// whose VALID tier->=2 anchor time is before the claimed compromise time".
/// This function never inspects anchors or wall-clock time itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompromiseClass {
    NotAffected,
    ValidPreClaim,
    TamperedPostPosition,
    IndeterminateWindow,
}

pub fn classify_compromise(
    chain: &KeyChain,
    record_position: u64,
    record_keyid: &str,
    anchored_before_claim: bool,
) -> CompromiseClass {
    let Some(compromise) = chain.compromise() else {
        return CompromiseClass::NotAffected;
    };
    // `record_keyid` is untrusted caller input; decode defensively (byte
    // comparison, not string comparison, so hex-case differences can't
    // hide a match) rather than assume it is well-formed hex.
    let Ok(record_key_bytes) = hex_to_32(record_keyid) else {
        return CompromiseClass::NotAffected;
    };
    if record_key_bytes != compromise.compromised_key {
        return CompromiseClass::NotAffected;
    }
    // Sealing is positional and unconditional: a record at or past P is
    // TAMPERED no matter what anchor it claims. P itself is the compromise
    // declaration's own slot — the journal seals AT P, so nothing signed by
    // the compromised key can legitimately occupy or follow that position
    // (controller ruling, Task 4 fix round 1: `>=`, not `>`).
    if record_position >= compromise.position {
        return CompromiseClass::TamperedPostPosition;
    }
    if anchored_before_claim {
        CompromiseClass::ValidPreClaim
    } else {
        CompromiseClass::IndeterminateWindow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::sign_envelope;
    use ed25519_dalek::{SigningKey, VerifyingKey};

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn hex32(vk: &VerifyingKey) -> String {
        hex::encode(vk.to_bytes())
    }

    /// Builds a genesis key-record envelope line, self-signed by `sk`.
    /// `recovery` is the optional declared cold recovery key.
    fn genesis_line(sk: &SigningKey, recovery: Option<&VerifyingKey>) -> Vec<u8> {
        let payload = serde_json::to_vec(&serde_json::json!({
            "kind": "genesis",
            "position": 0,
            "signing_key": hex32(&sk.verifying_key()),
            "recovery_key": recovery.map(hex32),
            "predecessor_genesis": null,
            "key_prev": hex::encode(GENESIS_PREV),
        }))
        .expect("static JSON always serializes");
        sign_envelope(sk, &hex32(&sk.verifying_key()), PT_KEYRECORD, &payload).to_line()
    }

    /// Builds a rotation record at `position`, signed by `predecessor`,
    /// declaring `new`'s public key as the new signing key. `key_prev` is
    /// the caller-supplied running key-chain head (honest by default; tests
    /// that attack `key_prev` pass a tampered value).
    fn rotation_line(
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

    /// Builds a compromise record at `position`, signed by `signer` (pass
    /// the true recovery key for a legitimate record, or any other key to
    /// attack the authorization check), naming `compromised` as the
    /// compromised key and `claimed_compromise_time` as `c`.
    fn compromise_line(
        signer: &SigningKey,
        compromised: &VerifyingKey,
        position: u64,
        c: &str,
        key_prev: &[u8; 32],
    ) -> Vec<u8> {
        let payload = serde_json::to_vec(&serde_json::json!({
            "kind": "compromise",
            "position": position,
            "compromised_key": hex32(compromised),
            "claimed_compromise_time": c,
            "key_prev": hex::encode(key_prev),
        }))
        .expect("static JSON always serializes");
        sign_envelope(
            signer,
            &hex32(&signer.verifying_key()),
            PT_KEYRECORD,
            &payload,
        )
        .to_line()
    }

    /// Running key-chain head after consuming `line` from `head`.
    fn advance(head: [u8; 32], line: &[u8]) -> [u8; 32] {
        head_hash(&head, &leaf_hash(line))
    }

    fn as_slices(lines: &[Vec<u8>]) -> Vec<&[u8]> {
        lines.iter().map(|l| l.as_slice()).collect()
    }

    // ---- genesis-only happy path ----

    #[test]
    fn genesis_only_chain_verifies() {
        let sk = key(1);
        let lines = vec![genesis_line(&sk, None)];
        let chain = verify_key_chain(&as_slices(&lines)).expect("genesis-only chain must verify");
        assert!(chain.compromise().is_none());
        assert!(chain.recovery_key().is_none());
        assert_eq!(
            key_at_position(&chain, 0).copied(),
            Some(sk.verifying_key())
        );
        assert_eq!(
            key_at_position(&chain, 1_000_000).copied(),
            Some(sk.verifying_key())
        );
    }

    #[test]
    fn genesis_with_recovery_key_verifies() {
        let sk = key(1);
        let recovery = key(9);
        let lines = vec![genesis_line(&sk, Some(&recovery.verifying_key()))];
        let chain = verify_key_chain(&as_slices(&lines)).expect("must verify");
        assert_eq!(
            chain.recovery_key().copied(),
            Some(recovery.verifying_key())
        );
    }

    // ---- 3.1: rotation not signed by predecessor -> UnauthorizedRotation ----

    #[test]
    fn rotation_not_signed_by_predecessor_is_unauthorized() {
        let sk1 = key(1);
        let sk2 = key(2);
        let impostor = key(66);
        let genesis = genesis_line(&sk1, None);
        let head = advance(GENESIS_PREV, &genesis);
        // Correct position, correct key_prev, correct declared new key —
        // only the ACTUAL signer is wrong (impostor instead of sk1).
        let bad_rotation = rotation_line(&impostor, &sk2, 10, &head);
        let lines = vec![genesis, bad_rotation];
        let err = verify_key_chain(&as_slices(&lines)).unwrap_err();
        assert_eq!(err, KeyChainError::UnauthorizedRotation { position: 10 });
    }

    #[test]
    fn honest_rotation_verifies_and_updates_authoritative_key() {
        let sk1 = key(1);
        let sk2 = key(2);
        let genesis = genesis_line(&sk1, None);
        let head = advance(GENESIS_PREV, &genesis);
        let rotation = rotation_line(&sk1, &sk2, 10, &head);
        let lines = vec![genesis, rotation];
        let chain = verify_key_chain(&as_slices(&lines)).expect("honest rotation must verify");
        assert_eq!(key_at_position(&chain, 9).copied(), Some(sk1.verifying_key()));
        assert_eq!(
            key_at_position(&chain, 10).copied(),
            Some(sk2.verifying_key())
        );
    }

    // ---- 3.2: compromise signed by a key that is NOT the recovery key ----

    #[test]
    fn compromise_not_signed_by_recovery_key_is_unauthorized() {
        let sk1 = key(1);
        let recovery = key(9);
        let impostor = key(66);
        let genesis = genesis_line(&sk1, Some(&recovery.verifying_key()));
        let head = advance(GENESIS_PREV, &genesis);
        let bad_compromise =
            compromise_line(&impostor, &sk1.verifying_key(), 5, "2026-01-01T00:00:00Z", &head);
        let lines = vec![genesis, bad_compromise];
        let err = verify_key_chain(&as_slices(&lines)).unwrap_err();
        assert_eq!(err, KeyChainError::UnauthorizedControlRecord { position: 5 });
    }

    // ---- 3.3: compromise when genesis declared no recovery key ----

    #[test]
    fn compromise_with_no_recovery_key_declared_is_unauthorized() {
        let sk1 = key(1);
        let genesis = genesis_line(&sk1, None); // no recovery key
        let head = advance(GENESIS_PREV, &genesis);
        // Signed by sk1 itself (the only key available) — still must be
        // rejected: no recovery key exists to authorize ANY compromise
        // record, full stop.
        let compromise =
            compromise_line(&sk1, &sk1.verifying_key(), 5, "2026-01-01T00:00:00Z", &head);
        let lines = vec![genesis, compromise];
        let err = verify_key_chain(&as_slices(&lines)).unwrap_err();
        assert_eq!(err, KeyChainError::UnauthorizedControlRecord { position: 5 });
    }

    // ---- fix round 1, finding 3: claimed_compromise_time is validated
    // (RFC 3339) at the verify_key_chain boundary, not accepted as a raw
    // unvalidated string ----

    #[test]
    fn compromise_with_non_rfc3339_claimed_compromise_time_is_invalid() {
        let sk1 = key(1);
        let recovery = key(9);
        let genesis = genesis_line(&sk1, Some(&recovery.verifying_key()));
        let head = advance(GENESIS_PREV, &genesis);
        // Correctly signed, correctly linked — only the timestamp is bad.
        let compromise =
            compromise_line(&recovery, &sk1.verifying_key(), 5, "not-a-timestamp", &head);
        let lines = vec![genesis, compromise];
        let err = verify_key_chain(&as_slices(&lines)).unwrap_err();
        assert!(
            matches!(err, KeyChainError::InvalidCompromiseTime { position: 5, .. }),
            "expected InvalidCompromiseTime, got {err:?}"
        );
    }

    fn build_compromised_chain() -> (KeyChain, SigningKey, u64, &'static str) {
        let sk1 = key(1);
        let recovery = key(9);
        let genesis = genesis_line(&sk1, Some(&recovery.verifying_key()));
        let head = advance(GENESIS_PREV, &genesis);
        let claim_time = "2026-06-01T00:00:00Z";
        let compromise_position = 100u64;
        let compromise = compromise_line(
            &recovery,
            &sk1.verifying_key(),
            compromise_position,
            claim_time,
            &head,
        );
        let lines = vec![genesis, compromise];
        let chain = verify_key_chain(&as_slices(&lines)).expect("legitimate compromise must verify");
        (chain, sk1, compromise_position, claim_time)
    }

    // ---- 3.4 / 3.10 (key-chain-level): journal sealed after compromise ----

    #[test]
    fn any_key_record_after_compromise_is_rejected_as_journal_sealed() {
        let sk1 = key(1);
        let sk3 = key(3);
        let recovery = key(9);
        let genesis = genesis_line(&sk1, Some(&recovery.verifying_key()));
        let mut head = advance(GENESIS_PREV, &genesis);
        let compromise = compromise_line(&recovery, &sk1.verifying_key(), 50, "2026-01-01T00:00:00Z", &head);
        head = advance(head, &compromise);
        // A further, otherwise perfectly legitimate rotation attempt
        // (correctly linked, correctly signed by the pre-compromise key) —
        // must STILL be rejected, because the chain is sealed.
        let trailing_rotation = rotation_line(&sk1, &sk3, 200, &head);
        let lines = vec![genesis, compromise, trailing_rotation];
        let err = verify_key_chain(&as_slices(&lines)).unwrap_err();
        assert_eq!(
            err,
            KeyChainError::JournalSealed {
                index: 2,
                sealed_at: 50
            }
        );
    }

    #[test]
    fn a_second_compromise_record_after_the_first_is_rejected_as_journal_sealed() {
        let sk1 = key(1);
        let recovery = key(9);
        let genesis = genesis_line(&sk1, Some(&recovery.verifying_key()));
        let mut head = advance(GENESIS_PREV, &genesis);
        let compromise1 =
            compromise_line(&recovery, &sk1.verifying_key(), 50, "2026-01-01T00:00:00Z", &head);
        head = advance(head, &compromise1);
        let compromise2 =
            compromise_line(&recovery, &sk1.verifying_key(), 80, "2026-02-01T00:00:00Z", &head);
        let lines = vec![genesis, compromise1, compromise2];
        let err = verify_key_chain(&as_slices(&lines)).unwrap_err();
        assert_eq!(
            err,
            KeyChainError::JournalSealed {
                index: 2,
                sealed_at: 50
            }
        );
    }

    // ---- classify_compromise matrix (rows 7, 8, 9) ----

    #[test]
    fn classify_no_compromise_is_not_affected() {
        let sk1 = key(1);
        let genesis = genesis_line(&sk1, None);
        let lines = vec![genesis];
        let chain = verify_key_chain(&as_slices(&lines)).expect("must verify");
        let keyid = hex32(&sk1.verifying_key());
        assert_eq!(
            classify_compromise(&chain, 5, &keyid, false),
            CompromiseClass::NotAffected
        );
    }

    #[test]
    fn classify_record_signed_by_a_different_key_is_not_affected() {
        let (chain, _sk1, p, _c) = build_compromised_chain();
        let unrelated = key(42);
        let keyid = hex32(&unrelated.verifying_key());
        assert_eq!(
            classify_compromise(&chain, p - 1, &keyid, true),
            CompromiseClass::NotAffected
        );
    }

    // 3.4: position > P -> TamperedPostPosition (regardless of anchoring).
    #[test]
    fn classify_record_after_compromise_position_is_tampered_post_position() {
        let (chain, sk1, p, _c) = build_compromised_chain();
        let keyid = hex32(&sk1.verifying_key());
        assert_eq!(
            classify_compromise(&chain, p + 1, &keyid, true),
            CompromiseClass::TamperedPostPosition
        );
        // Even a (forged) claim of anchoring can't rescue a post-position record.
        assert_eq!(
            classify_compromise(&chain, p + 1, &keyid, false),
            CompromiseClass::TamperedPostPosition
        );
    }

    // Fix round 1, finding 2 (controller ruling): position == P -> also
    // TamperedPostPosition, not just position > P. P is the compromise
    // declaration's own slot; the journal seals AT P, so nothing signed by
    // the compromised key can legitimately occupy that position either.
    // Even a (forged) claim of anchoring can't rescue it.
    #[test]
    fn classify_record_at_exact_compromise_position_is_tampered_post_position() {
        let (chain, sk1, p, _c) = build_compromised_chain();
        let keyid = hex32(&sk1.verifying_key());
        assert_eq!(
            classify_compromise(&chain, p, &keyid, true),
            CompromiseClass::TamperedPostPosition
        );
        assert_eq!(
            classify_compromise(&chain, p, &keyid, false),
            CompromiseClass::TamperedPostPosition
        );
    }

    // 3.5: position < P, anchored before claim -> ValidPreClaim.
    #[test]
    fn classify_record_before_compromise_anchored_before_claim_is_valid_pre_claim() {
        let (chain, sk1, p, _c) = build_compromised_chain();
        let keyid = hex32(&sk1.verifying_key());
        assert_eq!(
            classify_compromise(&chain, p - 1, &keyid, true),
            CompromiseClass::ValidPreClaim
        );
    }

    // 3.6: position < P, unanchored -> IndeterminateWindow.
    #[test]
    fn classify_record_before_compromise_unanchored_is_indeterminate() {
        let (chain, sk1, p, _c) = build_compromised_chain();
        let keyid = hex32(&sk1.verifying_key());
        assert_eq!(
            classify_compromise(&chain, p - 1, &keyid, false),
            CompromiseClass::IndeterminateWindow
        );
    }

    // 3.7: position < P, anchored but WITHIN [C, declaration] -> Indeterminate.
    // (Modeled exactly like 3.6/3.8: the caller has already determined the
    // anchor does NOT predate C, and passes anchored_before_claim=false —
    // classify_compromise itself never inspects the anchor time, by design.)
    #[test]
    fn classify_record_anchored_within_c_to_declaration_window_is_indeterminate() {
        let (chain, sk1, p, _c) = build_compromised_chain();
        let keyid = hex32(&sk1.verifying_key());
        assert_eq!(
            classify_compromise(&chain, p - 1, &keyid, false),
            CompromiseClass::IndeterminateWindow
        );
    }

    // 3.8 (REQUIRED, trust anchor for this task): detection-lag forgery.
    // A thief who stole K after the real compromise forges records at
    // positions < P and stamps them with anchors dated AFTER C (but before
    // the recovery-key holder got around to declaring the compromise).
    // Task 7 is responsible for computing `anchored_before_claim` correctly
    // from real anchor timestamps (an anchor dated >= C must yield `false`
    // here) — but this test locks in the property THIS function guarantees
    // unconditionally: there is NO input to classify_compromise that
    // produces ValidPreClaim except `anchored_before_claim == true`. A
    // forged anchor dated after C, correctly reported to this function as
    // `anchored_before_claim = false`, can NEVER come out the other end as
    // ValidPreClaim.
    #[test]
    fn classify_detection_lag_forgery_is_indeterminate_never_valid() {
        let (chain, sk1, p, _c) = build_compromised_chain();
        let keyid = hex32(&sk1.verifying_key());
        // Forged record sits well before P, exactly like a legitimate
        // pre-compromise record would — the ONLY distinguishing fact is the
        // anchor time, which the caller has already resolved to "not
        // before C" and passed in as `false`.
        let forged_position = p - 1;
        let got = classify_compromise(&chain, forged_position, &keyid, false);
        assert_eq!(got, CompromiseClass::IndeterminateWindow);
        assert_ne!(got, CompromiseClass::ValidPreClaim);
    }

    // 3.9 (REQUIRED): malicious recovery key sets claimed_compromise_time C
    // to the earliest representable instant ("genesis time"). The attack:
    // a coerced or malicious recovery-key holder picks a C so early that NO
    // anchor can ever predate it, hoping that either (a) the verifier
    // fabricates a false ValidPreClaim for every pre-compromise record
    // because "surely nothing could be before C" gets mishandled as
    // "anchored", or (b) the verifier over-corrects and casts legitimate
    // pre-compromise history as TamperedPostPosition. Neither must happen.
    // A real caller, unable to find any anchor before an already-minimal C,
    // can only ever pass `anchored_before_claim = false` — and this test
    // pins that the ENTIRE pre-P journal then lands on IndeterminateWindow,
    // never ValidPreClaim and never TamperedPostPosition.
    #[test]
    fn classify_malicious_recovery_key_claims_genesis_time_is_indeterminate_never_valid_or_tampered(
    ) {
        let sk1 = key(1);
        let recovery = key(9);
        let genesis = genesis_line(&sk1, Some(&recovery.verifying_key()));
        let head = advance(GENESIS_PREV, &genesis);
        // Earliest representable RFC 3339 instant — the malicious claim.
        let genesis_time = "0001-01-01T00:00:00Z";
        let compromise_position = 100u64;
        let compromise = compromise_line(
            &recovery,
            &sk1.verifying_key(),
            compromise_position,
            genesis_time,
            &head,
        );
        let lines = vec![genesis, compromise];
        let chain =
            verify_key_chain(&as_slices(&lines)).expect("a genesis-timed claim is still valid RFC3339");
        let keyid = hex32(&sk1.verifying_key());

        for position in [1u64, 2, 50, compromise_position - 1] {
            let got = classify_compromise(&chain, position, &keyid, false);
            assert_eq!(
                got,
                CompromiseClass::IndeterminateWindow,
                "position {position} (< P={compromise_position}) must be Indeterminate, got {got:?}"
            );
            assert_ne!(got, CompromiseClass::ValidPreClaim);
            assert_ne!(got, CompromiseClass::TamperedPostPosition);
        }
    }

    // ---- 3.11: key_at_position boundary (declaration position belongs to NEW key) ----

    #[test]
    fn key_at_position_boundary_declaration_position_belongs_to_new_key() {
        let sk1 = key(1);
        let sk2 = key(2);
        let genesis = genesis_line(&sk1, None);
        let head = advance(GENESIS_PREV, &genesis);
        let rotation = rotation_line(&sk1, &sk2, 50, &head);
        let lines = vec![genesis, rotation];
        let chain = verify_key_chain(&as_slices(&lines)).expect("must verify");

        assert_eq!(
            key_at_position(&chain, 49).copied(),
            Some(sk1.verifying_key()),
            "position 49 (just before declaration) is still the OLD key"
        );
        assert_eq!(
            key_at_position(&chain, 50).copied(),
            Some(sk2.verifying_key()),
            "position 50 (the declaration position itself) belongs to the NEW key"
        );
        assert_eq!(
            key_at_position(&chain, 51).copied(),
            Some(sk2.verifying_key())
        );
        // And the genesis boundary itself: position 0 belongs to genesis's key.
        assert_eq!(key_at_position(&chain, 0).copied(), Some(sk1.verifying_key()));
    }

    #[test]
    fn key_at_position_with_multiple_rotations_walks_every_boundary() {
        let sk1 = key(1);
        let sk2 = key(2);
        let sk3 = key(3);
        let genesis = genesis_line(&sk1, None);
        let mut head = advance(GENESIS_PREV, &genesis);
        let rotation1 = rotation_line(&sk1, &sk2, 30, &head);
        head = advance(head, &rotation1);
        let rotation2 = rotation_line(&sk2, &sk3, 70, &head);
        let lines = vec![genesis, rotation1, rotation2];
        let chain = verify_key_chain(&as_slices(&lines)).expect("must verify");

        for p in 0..30 {
            assert_eq!(key_at_position(&chain, p).copied(), Some(sk1.verifying_key()));
        }
        for p in 30..70 {
            assert_eq!(key_at_position(&chain, p).copied(), Some(sk2.verifying_key()));
        }
        for p in [70, 71, 1000] {
            assert_eq!(key_at_position(&chain, p).copied(), Some(sk3.verifying_key()));
        }
    }

    // ---- structural / malformed cases ----

    #[test]
    fn empty_chain_is_missing_genesis() {
        let err = verify_key_chain(&[]).unwrap_err();
        assert_eq!(err, KeyChainError::MissingGenesis);
    }

    #[test]
    fn chain_not_starting_with_genesis_is_missing_genesis() {
        let sk1 = key(1);
        let sk2 = key(2);
        let rotation = rotation_line(&sk1, &sk2, 1, &GENESIS_PREV);
        let err = verify_key_chain(&[rotation.as_slice()]).unwrap_err();
        assert_eq!(err, KeyChainError::MissingGenesis);
    }

    #[test]
    fn genesis_position_must_be_zero() {
        let sk = key(1);
        let payload = serde_json::to_vec(&serde_json::json!({
            "kind": "genesis",
            "position": 1,
            "signing_key": hex32(&sk.verifying_key()),
            "recovery_key": null,
            "predecessor_genesis": null,
            "key_prev": hex::encode(GENESIS_PREV),
        }))
        .unwrap();
        let line = sign_envelope(&sk, &hex32(&sk.verifying_key()), PT_KEYRECORD, &payload).to_line();
        let err = verify_key_chain(&[line.as_slice()]).unwrap_err();
        assert_eq!(err, KeyChainError::GenesisPositionMismatch { found: 1 });
    }

    #[test]
    fn genesis_key_prev_must_be_the_zero_anchor() {
        let sk = key(1);
        let bad_prev = [7u8; 32];
        let payload = serde_json::to_vec(&serde_json::json!({
            "kind": "genesis",
            "position": 0,
            "signing_key": hex32(&sk.verifying_key()),
            "recovery_key": null,
            "predecessor_genesis": null,
            "key_prev": hex::encode(bad_prev),
        }))
        .unwrap();
        let line = sign_envelope(&sk, &hex32(&sk.verifying_key()), PT_KEYRECORD, &payload).to_line();
        let err = verify_key_chain(&[line.as_slice()]).unwrap_err();
        assert_eq!(err, KeyChainError::KeyLinkMismatch { position: 0 });
    }

    #[test]
    fn genesis_not_self_signed_is_rejected() {
        let sk = key(1);
        let impostor = key(2);
        let payload = serde_json::to_vec(&serde_json::json!({
            "kind": "genesis",
            "position": 0,
            "signing_key": hex32(&sk.verifying_key()), // declares sk...
            "recovery_key": null,
            "predecessor_genesis": null,
            "key_prev": hex::encode(GENESIS_PREV),
        }))
        .unwrap();
        // ...but the envelope is actually signed by someone else.
        let line =
            sign_envelope(&impostor, &hex32(&sk.verifying_key()), PT_KEYRECORD, &payload).to_line();
        let err = verify_key_chain(&[line.as_slice()]).unwrap_err();
        assert_eq!(err, KeyChainError::GenesisNotSelfSigned);
    }

    #[test]
    fn duplicate_genesis_is_rejected() {
        let sk1 = key(1);
        let sk2 = key(2);
        let genesis1 = genesis_line(&sk1, None);
        let genesis2 = genesis_line(&sk2, None);
        let lines = vec![genesis1, genesis2];
        let err = verify_key_chain(&as_slices(&lines)).unwrap_err();
        assert_eq!(err, KeyChainError::DuplicateGenesis { index: 1 });
    }

    #[test]
    fn unknown_kind_is_rejected() {
        let sk1 = key(1);
        let genesis = genesis_line(&sk1, None);
        let head = advance(GENESIS_PREV, &genesis);
        let payload = serde_json::to_vec(&serde_json::json!({
            "kind": "retire",
            "position": 5,
            "key_prev": hex::encode(head),
        }))
        .unwrap();
        let bad = sign_envelope(&sk1, &hex32(&sk1.verifying_key()), PT_KEYRECORD, &payload).to_line();
        let lines = vec![genesis, bad];
        let err = verify_key_chain(&as_slices(&lines)).unwrap_err();
        assert_eq!(
            err,
            KeyChainError::UnknownKind {
                index: 1,
                kind: "retire".to_string()
            }
        );
    }

    #[test]
    fn non_increasing_rotation_position_is_rejected() {
        let sk1 = key(1);
        let sk2 = key(2);
        let sk3 = key(3);
        let genesis = genesis_line(&sk1, None);
        let mut head = advance(GENESIS_PREV, &genesis);
        let rotation1 = rotation_line(&sk1, &sk2, 30, &head);
        head = advance(head, &rotation1);
        // Same position as the previous rotation — must strictly increase.
        let rotation2 = rotation_line(&sk2, &sk3, 30, &head);
        let lines = vec![genesis, rotation1, rotation2];
        let err = verify_key_chain(&as_slices(&lines)).unwrap_err();
        assert_eq!(
            err,
            KeyChainError::PositionNotIncreasing {
                previous: 30,
                found: 30
            }
        );
    }

    #[test]
    fn rotation_key_prev_mismatch_is_rejected() {
        let sk1 = key(1);
        let sk2 = key(2);
        let genesis = genesis_line(&sk1, None);
        let bad_head = [0xAAu8; 32]; // not the honest running head
        let rotation = rotation_line(&sk1, &sk2, 10, &bad_head);
        let lines = vec![genesis, rotation];
        let err = verify_key_chain(&as_slices(&lines)).unwrap_err();
        assert_eq!(err, KeyChainError::KeyLinkMismatch { position: 10 });
    }

    #[test]
    fn malformed_json_is_malformed_not_a_panic() {
        let garbage: &[u8] = b"not json at all {{{";
        let err = verify_key_chain(&[garbage]).unwrap_err();
        assert!(matches!(err, KeyChainError::Malformed { index: 0, .. }));
    }

    #[test]
    fn unknown_field_in_genesis_payload_is_malformed() {
        let sk = key(1);
        let payload = serde_json::to_vec(&serde_json::json!({
            "kind": "genesis",
            "position": 0,
            "signing_key": hex32(&sk.verifying_key()),
            "recovery_key": null,
            "predecessor_genesis": null,
            "key_prev": hex::encode(GENESIS_PREV),
            "extra_field": true,
        }))
        .unwrap();
        let line = sign_envelope(&sk, &hex32(&sk.verifying_key()), PT_KEYRECORD, &payload).to_line();
        let err = verify_key_chain(&[line.as_slice()]).unwrap_err();
        assert!(matches!(err, KeyChainError::Malformed { index: 0, .. }));
    }

    // ---- proptest: half-open interval logic over an arbitrary rotation schedule ----

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]
        #[test]
        fn key_at_position_matches_brute_force_half_open_interval_lookup(
            gaps in proptest::collection::vec(1u64..50, 0..8),
            query_offset in 0u64..200,
        ) {
            // Build a genesis + one rotation per gap, at strictly increasing
            // positions (cumulative sums of `gaps`, each >= 1).
            let mut sks = vec![key(1)];
            for i in 0..gaps.len() {
                sks.push(key((i as u8).wrapping_add(2)));
            }
            let mut positions = vec![0u64];
            let mut acc = 0u64;
            for g in &gaps {
                acc += g;
                positions.push(acc);
            }

            let mut lines: Vec<Vec<u8>> = vec![genesis_line(&sks[0], None)];
            let mut head = advance(GENESIS_PREV, &lines[0]);
            for i in 0..gaps.len() {
                let predecessor = &sks[i];
                let new = &sks[i + 1];
                let position = positions[i + 1];
                let line = rotation_line(predecessor, new, position, &head);
                head = advance(head, &line);
                lines.push(line);
            }

            let chain = verify_key_chain(&as_slices(&lines)).expect("honestly-built chain must verify");

            // Brute-force expected key: the event with the LARGEST position <= query.
            let max_position = *positions.last().unwrap();
            let query = query_offset.min(max_position + 50);
            let expected_index = positions
                .iter()
                .rposition(|&p| p <= query)
                .expect("position 0 always satisfies p <= query for any u64 query");
            let expected_vk = sks[expected_index].verifying_key();

            let got = key_at_position(&chain, query).copied();
            prop_assert_eq!(got, Some(expected_vk));
        }
    }
}
