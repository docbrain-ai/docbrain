// SPDX-License-Identifier: MIT
//! Signed bundle manifest: the container's root of trust. `manifest.json`
//! is DSSE-signed (`PT_MANIFEST`) under the key valid, per the key chain,
//! at the position of the export checkpoint it certifies — the same
//! signer-resolution pattern `checkpoint.rs` uses for every checkpoint
//! (`key_at_position`), because the manifest is, functionally, one more
//! position-anchored signed statement, just about the bundle as a whole
//! rather than one record.
//!
//! ## Schema (pinned by this module — Task 11's exporter and Task 14's
//! Python verifier must match this exactly)
//!
//! ```json
//! {
//!   "scope": {"range": [start, end], "classes": ["..."], "spaces": ["..."] | null},
//!   "counts": {"records": N, "closure": N, "withheld_erased": N},
//!   "export_checkpoint": {"position": P, "head": "<hex32>", "count": N},
//!   "tool": {"exporter": "docbrain x.y.z"},
//!   "members": {"<name>": "<sha256 hex>", ...}
//! }
//! ```
//!
//! The brief's schema sketch leaves `export_checkpoint`'s shape as `{...}`
//! — this module pins it to exactly `{position, head, count}`: three of
//! `checkpoint::Checkpoint`'s four fields (`position`, `head`, `count`,
//! `at`), deliberately omitting `at` — NOT full parity with `Checkpoint`.
//! `position` is needed to resolve `key_at_position` for signer
//! resolution; `head`/`count` are what a downstream verifier (Task 7)
//! needs to cross-check the manifest's claimed checkpoint against the
//! bundle's own `checkpoints.jsonl` chain. Wall-clock `at` has no role
//! here — the manifest doesn't assert a time, only a chain position — so
//! it is intentionally not part of this schema.
//!
//! `manifest.json` is never listed in its own `members` map — nothing can
//! authenticate itself by hashing itself; its authenticity is its own DSSE
//! signature, verified first, from its exact bytes, before anything else
//! in the bundle is interpreted (design doc "order of operations").
//!
//! ## Verification sequencing (peek → resolve key → verify → re-parse)
//!
//! Resolving WHICH key should check the manifest's signature requires
//! knowing `export_checkpoint.position` — but that value lives INSIDE the
//! (not-yet-verified) payload. This mirrors `checkpoint.rs`'s own
//! resolution problem exactly, and uses the same fix: a tolerant,
//! `deny_unknown_fields`-free peek decodes JUST enough of the payload to
//! read `export_checkpoint.position` (used only for signer ROUTING, never
//! trusted as content), THEN [`verify_envelope`] does the real,
//! closed-schema-blind signature check, and only once that succeeds does
//! this module re-parse the FULL closed schema from the verified bytes.
//! Worst case an attacker causes a wrong-key lookup, which just fails
//! signature verification — the peek is never itself a trust boundary.

use crate::container::{ContainerError, ContainerReader};
use crate::envelope::{verify_envelope, EnvelopeError, PT_MANIFEST};
use crate::keys::{key_at_position, KeyChain};
use crate::strict::from_slice_strict;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// The one member name that is never itself listed in `members` (see
/// module docs).
pub const MANIFEST_MEMBER_NAME: &str = "manifest.json";

/// Every variant is a fail-closed outcome. `Missing` maps to
/// `CANNOT_VERIFY(container-profile)` (design doc row 21 — the fixed
/// manifest.json member the profile requires simply isn't there).
/// `Malformed` maps to `CANNOT_VERIFY(malformed)` (row 22). `Signature`
/// covers exactly one cause after the Task 7 un-collapse below (2026-08-24
/// controller ruling): the manifest's DSSE signature failed to verify
/// under the key valid, per the key chain, at `export_checkpoint.position`
/// — real evidence the manifest bytes were tampered post-signing, which
/// the verdict engine maps to `TAMPERED(manifest)` (row 11). A
/// wrong-payloadType/malformed/unsupported envelope at the manifest slot
/// is reported as `Malformed` instead, NOT `Signature` — see
/// [`verify_manifest`]'s doc comment for why collapsing that distinction
/// would mislabel a structural/format issue as tampering.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest.json member missing from container")]
    Missing,
    #[error("manifest payload malformed: {0}")]
    Malformed(String),
    #[error("manifest signature invalid")]
    Signature,
}

/// Fail-closed member-hash outcomes (design doc rows 12/21): a hash
/// mismatch is content tampering (row 12, `TAMPERED`); an unlisted or
/// missing member is a container-profile violation (row 21,
/// `CANNOT_VERIFY`) — both are reported here as distinct variants so the
/// verdict engine (Task 7) can map each to its correct row. This module
/// does not itself rank or collapse them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MemberError {
    #[error("member {name:?} hash does not match the signed manifest")]
    HashMismatch { name: String },
    #[error("member {name:?} present in the container but absent from the signed manifest")]
    UnlistedMember { name: String },
    #[error("member {name:?} listed in the signed manifest but absent from the container")]
    MissingMember { name: String },
}

/// Fields are private: parse-at-the-boundary (repo Rust discipline) — the
/// only way to obtain a `Scope` is as part of a [`Manifest`], and the only
/// way to obtain a `Manifest` is [`verify_manifest`], so a `Scope` in hand
/// is proof its bytes came from a DSSE-verified payload, never a
/// hand-built struct literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    range: (u64, u64),
    classes: Vec<String>,
    spaces: Option<Vec<String>>,
}

impl Scope {
    pub fn range(&self) -> (u64, u64) {
        self.range
    }
    pub fn classes(&self) -> &[String] {
        &self.classes
    }
    pub fn spaces(&self) -> Option<&[String]> {
        self.spaces.as_deref()
    }
}

/// See [`Scope`]'s doc comment: private fields, same reasoning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    records: u64,
    closure: u64,
    withheld_erased: u64,
}

impl Counts {
    pub fn records(&self) -> u64 {
        self.records
    }
    pub fn closure(&self) -> u64 {
        self.closure
    }
    pub fn withheld_erased(&self) -> u64 {
        self.withheld_erased
    }
}

/// See [`Scope`]'s doc comment: private fields, same reasoning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportCheckpointRef {
    position: u64,
    head: [u8; 32],
    count: u64,
}

impl ExportCheckpointRef {
    pub fn position(&self) -> u64 {
        self.position
    }
    pub fn head(&self) -> [u8; 32] {
        self.head
    }
    pub fn count(&self) -> u64 {
        self.count
    }
}

/// A verified bundle manifest. Only constructible via [`verify_manifest`]
/// — every field is private, so this is not just documentation: there is
/// no struct-literal syntax anywhere outside this module that can produce
/// a `Manifest`, meaning its existence IS the proof the DSSE signature
/// already checked out under the key valid at `export_checkpoint.position`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    scope: Scope,
    counts: Counts,
    export_checkpoint: ExportCheckpointRef,
    tool_exporter: String,
    members: BTreeMap<String, [u8; 32]>,
}

impl Manifest {
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn counts(&self) -> Counts {
        self.counts
    }
    pub fn export_checkpoint(&self) -> ExportCheckpointRef {
        self.export_checkpoint
    }
    pub fn tool_exporter(&self) -> &str {
        &self.tool_exporter
    }
    pub fn members(&self) -> &BTreeMap<String, [u8; 32]> {
        &self.members
    }
}

fn hex_to_32(s: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(s).map_err(|e| format!("invalid hex: {e}"))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("expected 32 bytes, got {}", bytes.len()))
}

/// Tolerant, minimal peek at JUST the DSSE envelope's `payload` field and
/// then JUST `export_checkpoint.position` inside it — see module docs.
#[derive(Deserialize)]
struct EnvelopePeek {
    payload: String,
}

#[derive(Deserialize)]
struct PositionPeek {
    export_checkpoint: PositionOnly,
}

#[derive(Deserialize)]
struct PositionOnly {
    position: u64,
}

fn peek_position(line: &[u8]) -> Result<u64, ManifestError> {
    let envelope: EnvelopePeek = from_slice_strict(line)
        .map_err(|e| ManifestError::Malformed(format!("envelope JSON: {e}")))?;
    let payload_bytes = STANDARD
        .decode(envelope.payload.as_bytes())
        .map_err(|e| ManifestError::Malformed(format!("payload base64: {e}")))?;
    let peek: PositionPeek = from_slice_strict(&payload_bytes)
        .map_err(|e| ManifestError::Malformed(format!("payload JSON (position peek): {e}")))?;
    Ok(peek.export_checkpoint.position)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeWire {
    range: (u64, u64),
    classes: Vec<String>,
    spaces: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CountsWire {
    records: u64,
    closure: u64,
    withheld_erased: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportCheckpointWire {
    position: u64,
    head: String,
    count: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolWire {
    exporter: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestWire {
    scope: ScopeWire,
    counts: CountsWire,
    export_checkpoint: ExportCheckpointWire,
    tool: ToolWire,
    members: BTreeMap<String, String>,
}

/// Locates `manifest.json`, resolves its signer via `key_at_position`
/// against the (already-verified) key chain, DSSE-verifies it under
/// `PT_MANIFEST`, and parses the closed schema from the exact verified
/// payload bytes. Nothing about the manifest's CONTENT is trusted before
/// the signature check succeeds (see module docs on the peek/verify/
/// re-parse sequencing).
pub fn verify_manifest(
    reader: &ContainerReader,
    keys: &KeyChain,
) -> Result<Manifest, ManifestError> {
    let line = reader
        .member_bytes(MANIFEST_MEMBER_NAME)
        .map_err(|_: ContainerError| ManifestError::Missing)?;

    let position = peek_position(line)?;
    // `key_at_position` returning `None` is structurally unreachable for
    // any chain `verify_key_chain` produced (genesis always covers
    // position 0, and every `u64` position is `>= 0`) — routed to
    // `Signature` defensively rather than `.expect()`-ed away, matching
    // the no-unwrap-in-prod bar.
    let vk = key_at_position(keys, position).ok_or(ManifestError::Signature)?;
    // Un-collapsed (Task 7 controller ruling, 2026-08-24 review, extended
    // by the same fix applied to keys.rs/checkpoint.rs): only an actual
    // cryptographic authenticity failure is `Signature` (→ TAMPERED
    // (manifest), row 11); a wrong-payloadType/malformed/unsupported
    // envelope at the manifest slot is a format issue, not tampering, and
    // must map to CANNOT_VERIFY(malformed) instead — collapsing them would
    // mean a spliced wrong-payloadType manifest reads as TAMPERED.
    let payload = verify_envelope(line, PT_MANIFEST, vk).map_err(|e| match e {
        EnvelopeError::SignatureInvalid => ManifestError::Signature,
        other => ManifestError::Malformed(format!("manifest envelope: {other}")),
    })?;

    let wire: ManifestWire = from_slice_strict(&payload)
        .map_err(|e| ManifestError::Malformed(format!("manifest payload JSON: {e}")))?;

    let head = hex_to_32(&wire.export_checkpoint.head)
        .map_err(|reason| ManifestError::Malformed(format!("export_checkpoint.head: {reason}")))?;

    let mut members = BTreeMap::new();
    for (name, hex_hash) in wire.members {
        let hash = hex_to_32(&hex_hash)
            .map_err(|reason| ManifestError::Malformed(format!("members[{name:?}]: {reason}")))?;
        members.insert(name, hash);
    }

    Ok(Manifest {
        scope: Scope {
            range: wire.scope.range,
            classes: wire.scope.classes,
            spaces: wire.scope.spaces,
        },
        counts: Counts {
            records: wire.counts.records,
            closure: wire.counts.closure,
            withheld_erased: wire.counts.withheld_erased,
        },
        export_checkpoint: ExportCheckpointRef {
            position: wire.export_checkpoint.position,
            head,
            count: wire.export_checkpoint.count,
        },
        tool_exporter: wire.tool.exporter,
        members,
    })
}

/// After the manifest itself is verified: every OTHER member's bytes are
/// hashed (plain SHA-256, not the crate's domain-separated `content_hash`
/// — this is a container-integrity check, not part of the evidence
/// hash-chain) and compared against the manifest's declared hash BEFORE
/// anything in the container is interpreted (design doc "order of
/// operations"). `manifest.json` itself is skipped (see module docs).
pub fn verify_members(reader: &ContainerReader, manifest: &Manifest) -> Result<(), MemberError> {
    for name in reader.member_names() {
        if name == MANIFEST_MEMBER_NAME {
            continue;
        }
        let Some(expected) = manifest.members.get(name) else {
            return Err(MemberError::UnlistedMember { name: name.clone() });
        };
        // Structurally unreachable: `name` came from `member_names()`,
        // which only ever lists names `ContainerReader::open` already
        // validated and indexed under the SAME key — `member_bytes` on a
        // name `member_names` just returned cannot fail. Handled instead
        // of `.expect()`-ed to keep the no-unwrap-in-prod bar absolute.
        let Ok(bytes) = reader.member_bytes(name) else {
            return Err(MemberError::UnlistedMember { name: name.clone() });
        };
        let got: [u8; 32] = Sha256::digest(bytes).into();
        if &got != expected {
            return Err(MemberError::HashMismatch { name: name.clone() });
        }
    }

    for name in manifest.members.keys() {
        if reader.member_bytes(name).is_err() {
            return Err(MemberError::MissingMember { name: name.clone() });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::ContainerWriter;
    use crate::envelope::{sign_envelope, PT_KEYRECORD};
    use crate::hash::GENESIS_PREV;
    use ed25519_dalek::{SigningKey, VerifyingKey};

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn hex32(vk: &VerifyingKey) -> String {
        hex::encode(vk.to_bytes())
    }

    /// A single-key chain: `sk` is valid at every position from 0 onward
    /// (mirrors `checkpoint.rs`'s own local fixture helper — `keys.rs`'s
    /// equivalents are private to its own test module).
    fn single_key_chain(sk: &SigningKey) -> KeyChain {
        let payload = serde_json::to_vec(&serde_json::json!({
            "kind": "genesis",
            "position": 0,
            "signing_key": hex32(&sk.verifying_key()),
            "recovery_key": null,
            "predecessor_genesis": null,
            "key_prev": hex::encode(GENESIS_PREV),
        }))
        .unwrap();
        let genesis =
            sign_envelope(sk, &hex32(&sk.verifying_key()), PT_KEYRECORD, &payload).to_line();
        crate::keys::verify_key_chain(&[genesis.as_slice()]).expect("fixture chain must verify")
    }

    /// Builds a signed `manifest.json` envelope line for the given signer
    /// and export-checkpoint position, plus a members map. `head_bytes`
    /// lets tests supply an arbitrary (even non-hex-decodable-later)
    /// checkpoint head; honest callers pass a real 32-byte array.
    fn manifest_line(
        signer: &SigningKey,
        position: u64,
        head: &[u8; 32],
        members: &BTreeMap<String, [u8; 32]>,
    ) -> Vec<u8> {
        let members_json: serde_json::Map<String, serde_json::Value> = members
            .iter()
            .map(|(name, hash)| (name.clone(), serde_json::json!(hex::encode(hash))))
            .collect();
        let payload = serde_json::to_vec(&serde_json::json!({
            "scope": {"range": [0, 100], "classes": ["fragment"], "spaces": null},
            "counts": {"records": 5, "closure": 0, "withheld_erased": 0},
            "export_checkpoint": {
                "position": position,
                "head": hex::encode(head),
                "count": 5,
            },
            "tool": {"exporter": "docbrain 0.0.0-test"},
            "members": serde_json::Value::Object(members_json),
        }))
        .unwrap();
        sign_envelope(signer, &hex32(&signer.verifying_key()), PT_MANIFEST, &payload).to_line()
    }

    fn sha256_32(data: &[u8]) -> [u8; 32] {
        Sha256::digest(data).into()
    }

    /// Builds a full valid container: manifest.json (signed) + two data
    /// members, with `manifest.json`'s `members` map correctly declaring
    /// both. Returns the raw bytes plus the signer/keychain used, so tests
    /// can tamper from a known-good baseline.
    fn build_valid_bundle() -> (Vec<u8>, SigningKey, KeyChain) {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let epoch = b"line-1\nline-2\n".to_vec();
        let checkpoints = b"cp-1\n".to_vec();

        let mut members = BTreeMap::new();
        members.insert("journal/epoch-0.jsonl".to_string(), sha256_32(&epoch));
        members.insert("checkpoints.jsonl".to_string(), sha256_32(&checkpoints));

        let manifest = manifest_line(&sk, 0, &GENESIS_PREV, &members);

        let mut w = ContainerWriter::new();
        w.add_member(MANIFEST_MEMBER_NAME, manifest).unwrap();
        w.add_member("journal/epoch-0.jsonl", epoch).unwrap();
        w.add_member("checkpoints.jsonl", checkpoints).unwrap();
        let bytes = w.finish().unwrap();

        (bytes, sk, keys)
    }

    // ---- valid round trip ----

    #[test]
    fn valid_manifest_and_members_verify_end_to_end() {
        let (bytes, _sk, keys) = build_valid_bundle();
        let reader = ContainerReader::open(&bytes).expect("container must open");
        let manifest = verify_manifest(&reader, &keys).expect("manifest must verify");
        assert_eq!(manifest.export_checkpoint().position(), 0);
        assert_eq!(manifest.counts().records(), 5);
        assert_eq!(manifest.members().len(), 2);
        verify_members(&reader, &manifest).expect("members must verify");
    }

    // ---- empty archive -> no manifest ----

    #[test]
    fn empty_archive_has_no_manifest() {
        let w = ContainerWriter::new();
        let bytes = w.finish().unwrap();
        let reader = ContainerReader::open(&bytes).expect("empty archive is structurally valid");
        let err = verify_manifest(&reader, &single_key_chain(&key(1))).unwrap_err();
        assert_eq!(err, ManifestError::Missing);
    }

    // ---- manifest signature flip ----

    #[test]
    fn flipped_manifest_signature_byte_is_rejected() {
        let (bytes, _sk, keys) = build_valid_bundle();
        let reader = ContainerReader::open(&bytes).expect("container must open");
        let line = reader.member_bytes(MANIFEST_MEMBER_NAME).unwrap();
        let mut wire: serde_json::Value = serde_json::from_slice(line).unwrap();
        let sig_b64 = wire["sig"].as_str().unwrap().to_string();
        let mut sig_bytes = STANDARD.decode(sig_b64).unwrap();
        sig_bytes[0] ^= 0xFF;
        wire["sig"] = serde_json::json!(STANDARD.encode(sig_bytes));
        let tampered_line = serde_json::to_vec(&wire).unwrap();

        // Re-embed the tampered manifest into a fresh container (so the
        // container-level byte-agreement checks pass and only the DSSE
        // signature itself is under test).
        let mut w = ContainerWriter::new();
        w.add_member(MANIFEST_MEMBER_NAME, tampered_line).unwrap();
        let bytes2 = w.finish().unwrap();
        let reader2 = ContainerReader::open(&bytes2).unwrap();
        let err = verify_manifest(&reader2, &keys).unwrap_err();
        assert_eq!(err, ManifestError::Signature);
    }

    #[test]
    fn manifest_signed_by_an_unrelated_key_is_rejected() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let impostor = key(66);
        let members = BTreeMap::new();
        let manifest = manifest_line(&impostor, 0, &GENESIS_PREV, &members);
        let mut w = ContainerWriter::new();
        w.add_member(MANIFEST_MEMBER_NAME, manifest).unwrap();
        let bytes = w.finish().unwrap();
        let reader = ContainerReader::open(&bytes).unwrap();
        let err = verify_manifest(&reader, &keys).unwrap_err();
        assert_eq!(err, ManifestError::Signature);
    }

    // ---- member byte flip -> HashMismatch ----

    #[test]
    fn flipped_member_byte_is_hash_mismatch() {
        let (bytes, _sk, keys) = build_valid_bundle();
        let reader = ContainerReader::open(&bytes).unwrap();
        let manifest = verify_manifest(&reader, &keys).unwrap();

        // Rebuild the same bundle but flip one byte of journal/epoch-0.jsonl
        // AFTER the manifest was computed against the ORIGINAL bytes —
        // i.e. the manifest still declares the hash of the honest content.
        let mut w = ContainerWriter::new();
        let manifest_line = reader.member_bytes(MANIFEST_MEMBER_NAME).unwrap().to_vec();
        w.add_member(MANIFEST_MEMBER_NAME, manifest_line).unwrap();
        let mut tampered_epoch = reader.member_bytes("journal/epoch-0.jsonl").unwrap().to_vec();
        tampered_epoch[0] ^= 0xFF;
        w.add_member("journal/epoch-0.jsonl", tampered_epoch).unwrap();
        w.add_member(
            "checkpoints.jsonl",
            reader.member_bytes("checkpoints.jsonl").unwrap().to_vec(),
        )
        .unwrap();
        let bytes2 = w.finish().unwrap();
        let reader2 = ContainerReader::open(&bytes2).unwrap();
        // Manifest signature still verifies (unrelated bytes changed).
        let manifest2 = verify_manifest(&reader2, &keys).expect("manifest itself is untouched");
        assert_eq!(manifest2, manifest);
        let err = verify_members(&reader2, &manifest2).unwrap_err();
        assert_eq!(
            err,
            MemberError::HashMismatch {
                name: "journal/epoch-0.jsonl".to_string()
            }
        );
    }

    // ---- member present in container, absent from manifest ----

    #[test]
    fn member_present_but_unlisted_in_manifest_is_rejected() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let mut members = BTreeMap::new();
        members.insert("checkpoints.jsonl".to_string(), sha256_32(b"cp"));
        let manifest_env = manifest_line(&sk, 0, &GENESIS_PREV, &members);

        let mut w = ContainerWriter::new();
        w.add_member(MANIFEST_MEMBER_NAME, manifest_env).unwrap();
        w.add_member("checkpoints.jsonl", b"cp".to_vec()).unwrap();
        // Extra member NOT declared in the manifest's members map.
        w.add_member("trust/keys.jsonl", b"extra".to_vec()).unwrap();
        let bytes = w.finish().unwrap();
        let reader = ContainerReader::open(&bytes).unwrap();
        let manifest = verify_manifest(&reader, &keys).unwrap();
        let err = verify_members(&reader, &manifest).unwrap_err();
        assert_eq!(
            err,
            MemberError::UnlistedMember {
                name: "trust/keys.jsonl".to_string()
            }
        );
    }

    // ---- member listed in manifest, absent from container ----

    #[test]
    fn member_listed_but_missing_from_container_is_rejected() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let mut members = BTreeMap::new();
        members.insert("checkpoints.jsonl".to_string(), sha256_32(b"cp"));
        // Manifest declares a hash for a member that is never added.
        members.insert("trust/keys.jsonl".to_string(), sha256_32(b"never-added"));
        let manifest_env = manifest_line(&sk, 0, &GENESIS_PREV, &members);

        let mut w = ContainerWriter::new();
        w.add_member(MANIFEST_MEMBER_NAME, manifest_env).unwrap();
        w.add_member("checkpoints.jsonl", b"cp".to_vec()).unwrap();
        let bytes = w.finish().unwrap();
        let reader = ContainerReader::open(&bytes).unwrap();
        let manifest = verify_manifest(&reader, &keys).unwrap();
        let err = verify_members(&reader, &manifest).unwrap_err();
        assert_eq!(
            err,
            MemberError::MissingMember {
                name: "trust/keys.jsonl".to_string()
            }
        );
    }

    // ---- malformed manifest payload (peek stage) ----

    #[test]
    fn manifest_payload_missing_export_checkpoint_is_malformed() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let payload = serde_json::to_vec(&serde_json::json!({
            "scope": {"range": [0, 1], "classes": [], "spaces": null},
            "counts": {"records": 0, "closure": 0, "withheld_erased": 0},
            // export_checkpoint deliberately omitted
            "tool": {"exporter": "x"},
            "members": {},
        }))
        .unwrap();
        let line = sign_envelope(&sk, &hex32(&sk.verifying_key()), PT_MANIFEST, &payload).to_line();
        let mut w = ContainerWriter::new();
        w.add_member(MANIFEST_MEMBER_NAME, line).unwrap();
        let bytes = w.finish().unwrap();
        let reader = ContainerReader::open(&bytes).unwrap();
        let err = verify_manifest(&reader, &keys).unwrap_err();
        assert!(matches!(err, ManifestError::Malformed(_)), "{err:?}");
    }

    #[test]
    fn manifest_payload_with_unknown_field_after_verification_is_malformed() {
        let sk = key(1);
        let keys = single_key_chain(&sk);
        let payload = serde_json::to_vec(&serde_json::json!({
            "scope": {"range": [0, 1], "classes": [], "spaces": null},
            "counts": {"records": 0, "closure": 0, "withheld_erased": 0},
            "export_checkpoint": {"position": 0, "head": hex::encode(GENESIS_PREV), "count": 0},
            "tool": {"exporter": "x"},
            "members": {},
            "extra_unknown_field": true,
        }))
        .unwrap();
        let line = sign_envelope(&sk, &hex32(&sk.verifying_key()), PT_MANIFEST, &payload).to_line();
        let mut w = ContainerWriter::new();
        w.add_member(MANIFEST_MEMBER_NAME, line).unwrap();
        let bytes = w.finish().unwrap();
        let reader = ContainerReader::open(&bytes).unwrap();
        let err = verify_manifest(&reader, &keys).unwrap_err();
        assert!(matches!(err, ManifestError::Malformed(_)), "{err:?}");
    }
}
