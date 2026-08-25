// SPDX-License-Identifier: MIT
//! `BundleBuilder`: the ONLY writer of `.dbev` bundles in the workspace
//! (Task 7 brief). Used by this crate's own pipeline tests, the golden
//! corpus (Task 15), the mutation/fuzz gates (Task 16), and the real
//! exporter (Task 11, which wraps this builder rather than hand-rolling
//! container bytes again).
//!
//! Two build phases, deliberately kept separate:
//! 1. **Recipe** — chainable `with_*`/`add_*` configuration methods that
//!    describe an HONEST bundle (keys, rotations, records, content,
//!    erasures, anchors). Erasure records are appended to the journal
//!    AFTER every real record (positions `N+1..N+E`), so they are ordinary
//!    later entries in the SAME hash chain, not out-of-band extras — this
//!    is what keeps checkpoint/manifest bookkeeping (`count`, `head`,
//!    `scope.range`) internally consistent even when erasures are present.
//! 2. **Mutation hooks** — named methods, one per taxonomy row this task
//!    can reach (`tamper_record`, `forge_position`, `duplicate_member`,
//!    ...), each queuing a specific, documented deviation from the honest
//!    recipe. `build()` always constructs the honest bundle FIRST, then
//!    applies every queued mutation — this is what lets a single call
//!    combine two independent mutations (e.g. a tampered record AND a
//!    malformed anchor) for the required combo test.
//!
//! Every signed line's payload byte-for-byte matches the closed schemas
//! `chain.rs`/`keys.rs`/`checkpoint.rs`/`manifest.rs` already pin;
//! `content/<position>` blobs are this module's own design decision (no
//! Task 1-6 module wires content addressing) — `salt(32 bytes) ||
//! content_bytes`, keyed by the record's decimal `position` (there is no
//! separate record-id field in `chain::RecordHeader`, and position is
//! already the bundle's natural unique identifier for a record).

use crate::container::{ContainerWriter, CDFH_SIG, EOCD_SIG, GPBF_UTF8, LFH_SIG, METHOD_STORE};
use crate::envelope::{sign_envelope, PT_CHECKPOINT, PT_KEYRECORD, PT_MANIFEST, PT_RECORD};
use crate::hash::{content_hash, head_hash, leaf_hash, GENESIS_PREV};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::{SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

const MANIFEST_MEMBER_NAME: &str = "manifest.json";

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn hex32(vk: &VerifyingKey) -> String {
    hex::encode(vk.to_bytes())
}

/// Deterministic 32 bytes, derived from a label and a counter — a
/// test-fixture salt generator, not a CSPRNG (no `rand` dependency needed;
/// this module never uses these bytes as anything but a content salt or a
/// deliberately-unrelated test keypair seed).
fn derived_32(label: &str, counter: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(label.as_bytes());
    h.update(counter.to_le_bytes());
    h.finalize().into()
}

/// Flips one byte (XOR 0x01) at the position immediately before the LAST
/// occurrence of `needle` ends within `haystack` — used to corrupt a
/// signed JSON payload's free-form string content in a way that (a) stays
/// syntactically valid JSON (the flipped byte is inside a quoted string,
/// never a structural character) and (b) never touches a field this
/// crate's own closed-schema parsers interpret (position/prev_head/
/// content_hash/keyid/hex fields), so the corruption is visible ONLY to
/// signature verification — exactly what isolates a row-2/row-11 "tampered
/// signature" finding from a row-22 "malformed" one.
fn flip_last_byte_of(haystack: &mut [u8], needle: &[u8]) {
    let pos = haystack
        .windows(needle.len())
        .rposition(|w| w == needle)
        .unwrap_or_else(|| panic!("needle {needle:?} not found in payload"));
    let idx = pos + needle.len() - 1;
    haystack[idx] ^= 0x01;
}

fn join_lines(lines: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for line in lines {
        out.extend_from_slice(line);
        out.push(b'\n');
    }
    out
}

/// One record in the recipe, positions auto-assigned 1..=N in add order.
struct RecordSpec {
    position: u64,
    class: String,
    kind: String,
    at: String,
    content: Option<Vec<u8>>,
    /// Row-3 mutation: sign with the key valid at THIS position instead of
    /// the record's own declared `position`.
    sign_as_of_position: Option<u64>,
    /// Row-16 mutation: sign with this specific key, never part of the
    /// chain, instead of any chain key.
    sign_with_foreign_key: Option<SigningKey>,
    /// Row-17 mutation: emit the multi-signature DSSE wire form.
    multi_sig: bool,
    /// Row-22 mutation: sign under `PT_CHECKPOINT` instead of `PT_RECORD`.
    wrong_payload_type: bool,
    /// Row-2 mutation: flip a byte in the signed payload post-hoc.
    tamper: bool,
    /// Row-4 mutation: declare a deliberately wrong `prev_head`, signed
    /// honestly over that lie — the record's OWN signature stays valid
    /// (isolating this from row 2), but the declared link no longer
    /// matches the chain's actual running head, so `walk_chain` fails
    /// fast with `LinkMismatch` exactly at this record's position.
    forge_prev_head: bool,
    /// Test-support (strict-JSON parity vectors): RAW bytes spliced verbatim
    /// as the record payload's `body` value, in place of the honest
    /// `{"seq": N}`. Signed and member-hashed over these exact bytes at build
    /// time, so a malformed body (duplicate key, out-of-range number, deep
    /// nesting — none of which `serde_json::json!` can emit) reaches the
    /// verifier's record PARSE rather than a member-hash mismatch. `Vec<u8>`
    /// (not `String`) so a raw non-UTF-8 byte can be injected too.
    raw_body: Option<Vec<u8>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ErasureMode {
    /// Content removed, honest erasure record present — VALID/withheld
    /// (row 13).
    Honest,
    /// Content removed, erasure record suppressed — bundle-incomplete
    /// (row 14).
    DropErasureRecord,
    /// Erasure record present, content ALSO left physically present —
    /// erasure-inconsistent (row 15).
    KeepContentDespiteErasure,
    /// Content removed, honest erasure record present, but the erasure
    /// record's OWN position is placed OUTSIDE the exported range (in
    /// `journal/closure.jsonl` rather than `journal/epoch-0.jsonl`) — the
    /// "erasure closure" case (row 13 via closure; Task 7 Ruling F1).
    HonestClosure,
}

#[derive(Clone, Copy)]
enum AnchorKind {
    Witness,
    Token,
}

struct AnchorSpec {
    kind: AnchorKind,
    checkpoint_position: u64,
    tsa_time: Option<String>,
    /// Row-18 plumbing: this anchor's own bytes fail to parse.
    malformed: bool,
    /// Row-19 plumbing: this anchor references a checkpoint position not
    /// present in the bundle's checkpoint chain.
    unlinked: bool,
    /// Raw bytes spliced verbatim before the anchor object's closing brace —
    /// the ONLY way to emit a DUPLICATE key (`serde_json::json!` collapses
    /// duplicates), INVALID UTF-8 inside an ignored field's value, or arbitrarily
    /// deep nesting. Test-support for the anchor Rust<->Python parity vectors;
    /// the manifest hashes the spliced bytes at build time so the anchor reaches
    /// the verifier's parse rather than a member-hash mismatch. `Vec<u8>` (not
    /// `String`) so a raw non-UTF-8 byte can be injected.
    extra_raw: Option<Vec<u8>>,
}

/// See module docs.
pub struct BundleBuilder {
    genesis: SigningKey,
    recovery: Option<SigningKey>,
    /// `(position, new_key)`, strictly increasing by position.
    rotations: Vec<(u64, SigningKey)>,
    corrupt_rotation_signer_at: Option<u64>,
    /// `(position, claimed_compromise_time)`.
    compromise: Option<(u64, String)>,
    corrupt_compromise_signer: bool,
    records: Vec<RecordSpec>,
    erase_positions: HashMap<u64, ErasureMode>,
    anchors: Vec<AnchorSpec>,
    mismatched_manifest_counts: bool,
    mismatched_manifest_export_checkpoint: bool,
    tamper_manifest: bool,
    tamper_member: Option<String>,
    unlisted_container_member: bool,
    missing_container_member: Option<String>,
    duplicate_member: Option<String>,
    zero_records: bool,
    /// Row-24 mutation: the end checkpoint's wall-clock `at` is set
    /// EARLIER than the start checkpoint's — chain position/hash order
    /// stays authoritative and honestly signed; only the wall clock lies.
    backwards_checkpoint_clock: bool,
    /// F2 mutation: an extra, in-range erasure record whose `target` does
    /// not resolve to any record in the bundle.
    dangling_erasure_target: Option<u64>,
    /// F2 mutation, closure variant: an extra `journal/closure.jsonl` entry
    /// whose `target` does not resolve to any in-range record.
    dangling_closure_erasure_target: Option<u64>,
    /// Structural-violation mutation: overrides the declared `position` of
    /// the (single) closure erasure record so it falls INSIDE the exported
    /// range instead of strictly outside it.
    closure_record_position_override: Option<u64>,
    /// Row-2-in-closure mutation (test-plan case B4): flips a byte in the
    /// closure erasure record's signed payload post-signing.
    tamper_closure_record: bool,
    /// Row-22-in-closure mutation: the (single) closure erasure record
    /// declares `kind: "note"` instead of `kind: "erasure"` — closure.jsonl
    /// may only ever carry erasure records.
    closure_record_wrong_kind: bool,
    /// Closure-count cross-check mutation: the manifest's declared
    /// `counts.closure` disagrees with the actual number of
    /// `journal/closure.jsonl` entries.
    mismatched_manifest_closure_count: bool,
    /// Windowed-export recipe: the exported range starts AFTER this
    /// position instead of at genesis (0) — models a compliance re-export
    /// of a RECENT window when the full journal has more history before
    /// it. `None` (the default) preserves every existing test's exact
    /// full-range-from-genesis behavior.
    window_start: Option<u64>,
    /// Task-16 timestamp-grammar hook: override the SIGNED `at` wall-clock of
    /// the start and end checkpoints with arbitrary strings (e.g. nanosecond
    /// precision, a `:60` leap second, alternate offset widths). Because the
    /// override is applied before the checkpoint envelope is signed, the
    /// signature stays valid over the unusual timestamp, so the string reaches
    /// the verifier's RFC-3339 parse/compare stage exactly as an honest one
    /// would. `None` (the default) keeps the hardcoded times and lets
    /// `backwards_checkpoint_clock` win; `Some` overrides both.
    checkpoint_times: Option<(String, String)>,
}

impl Default for BundleBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl BundleBuilder {
    pub fn new() -> Self {
        BundleBuilder {
            genesis: key(1),
            recovery: None,
            rotations: Vec::new(),
            corrupt_rotation_signer_at: None,
            compromise: None,
            corrupt_compromise_signer: false,
            records: Vec::new(),
            erase_positions: HashMap::new(),
            anchors: Vec::new(),
            mismatched_manifest_counts: false,
            mismatched_manifest_export_checkpoint: false,
            tamper_manifest: false,
            tamper_member: None,
            unlisted_container_member: false,
            missing_container_member: None,
            duplicate_member: None,
            zero_records: false,
            backwards_checkpoint_clock: false,
            dangling_erasure_target: None,
            dangling_closure_erasure_target: None,
            closure_record_position_override: None,
            tamper_closure_record: false,
            closure_record_wrong_kind: false,
            mismatched_manifest_closure_count: false,
            window_start: None,
            checkpoint_times: None,
        }
    }

    // ---- recipe ----

    pub fn with_recovery_key(mut self) -> Self {
        self.recovery = Some(key(9));
        self
    }

    /// Build this bundle under a DISTINCT genesis signing key (a different
    /// self-signed TOFU root), modeling an entirely SEPARATE journal. Two
    /// builders that differ only in this seed produce bundles that share
    /// position numbers but NOT journal identity — exactly the pair the CLI's
    /// `--against` identity gate must treat as "different journals", never a
    /// fork. The default genesis is `key(1)`; pass any other seed (that no
    /// other part of the recipe reuses) for a second, unrelated journal.
    pub fn with_genesis_key_seed(mut self, seed: u8) -> Self {
        self.genesis = key(seed);
        self
    }

    /// Adds a rotation at `position`: a fresh key becomes authoritative
    /// from `position` onward (half-open interval, `keys.rs` convention).
    pub fn with_rotation(mut self, position: u64) -> Self {
        let seed = 2 + self.rotations.len() as u8;
        self.rotations.push((position, key(seed)));
        self
    }

    /// Declares a compromise, at key-chain position `position`, of
    /// whatever key [`Self::signer_for_position`] resolves at
    /// `position - 1` (the key that WAS authoritative right before the
    /// declaration), claimed compromised at `claimed_time` (RFC 3339).
    /// Requires [`Self::with_recovery_key`] first.
    pub fn with_compromise(mut self, position: u64, claimed_time: &str) -> Self {
        self.compromise = Some((position, claimed_time.to_string()));
        self
    }

    /// Row-6 mutation: sign the compromise record with a key that is NOT
    /// the declared recovery key.
    pub fn corrupt_compromise_signer(mut self) -> Self {
        self.corrupt_compromise_signer = true;
        self
    }

    /// Row-5 mutation: sign the rotation at `position` with a key that is
    /// NOT its true predecessor.
    pub fn corrupt_rotation_signer(mut self, position: u64) -> Self {
        self.corrupt_rotation_signer_at = Some(position);
        self
    }

    /// Appends one honestly-signed record (auto position = count so far +
    /// 1, `class: "evidence-record"`, `kind: "note"`, deterministic `at`).
    pub fn add_record(mut self) -> Self {
        let position = self.records.len() as u64 + 1;
        self.records.push(RecordSpec {
            position,
            class: "evidence-record".to_string(),
            kind: "note".to_string(),
            at: format!("2026-01-01T00:{:02}:00Z", position.min(59)),
            content: None,
            sign_as_of_position: None,
            sign_with_foreign_key: None,
            multi_sig: false,
            wrong_payload_type: false,
            tamper: false,
            forge_prev_head: false,
            raw_body: None,
        });
        self
    }

    pub fn add_records(mut self, n: u64) -> Self {
        for _ in 0..n {
            self = self.add_record();
        }
        self
    }

    /// Sets the given record's content (auto-computes `content_hash` from
    /// a fresh deterministic salt; the raw bytes are stored under
    /// `content/<position>` at build time). `position` must already exist.
    pub fn with_content(mut self, position: u64, content: &[u8]) -> Self {
        self.record_mut(position).content = Some(content.to_vec());
        self
    }

    /// Test-support (strict-JSON parity vectors): build the record at
    /// `position` with a RAW `body` value spliced verbatim (bytes) in place of
    /// the honest `{"seq": N}`. The record is signed and member-hashed over
    /// these exact bytes at build time, so a malformed body — a duplicate key,
    /// an out-of-range number, arbitrarily deep nesting (none of which
    /// `serde_json::json!` can emit) — reaches the verifier's record PARSE
    /// instead of tripping a member-hash mismatch first. `position` must exist.
    pub fn with_record_raw_body(mut self, position: u64, raw_body: &[u8]) -> Self {
        self.record_mut(position).raw_body = Some(raw_body.to_vec());
        self
    }

    /// Row-13 path: erase `position`'s content honestly (blob+salt
    /// removed, a validly-signed erasure record added) — VALID, listed
    /// `withheld-erased`.
    pub fn erase(mut self, position: u64) -> Self {
        self.erase_positions.insert(position, ErasureMode::Honest);
        self
    }

    /// Row-14 mutation: content removed but the erasure record is
    /// suppressed — nothing explains the absence.
    pub fn drop_erasure(mut self, position: u64) -> Self {
        self.erase_positions
            .insert(position, ErasureMode::DropErasureRecord);
        self
    }

    /// Row-15 mutation: an honest erasure record is added, but the
    /// content blob is ALSO left physically present (stale export /
    /// resurrection).
    pub fn keep_content_despite_erasure(mut self, position: u64) -> Self {
        self.erase_positions
            .insert(position, ErasureMode::KeepContentDespiteErasure);
        self
    }

    /// Row-13-via-closure path: erase `position`'s content honestly, but
    /// place the erasure record in `journal/closure.jsonl` with its OWN
    /// position placed strictly AFTER the exported range boundary (as if
    /// the erasure happened after this range was originally checkpointed,
    /// and this bundle is a later re-export of that older window) — VALID,
    /// listed `withheld-erased`, closure count 1.
    pub fn erase_via_closure(mut self, position: u64) -> Self {
        self.erase_positions.insert(position, ErasureMode::HonestClosure);
        self
    }

    pub fn with_anchor_witness(mut self, checkpoint_position: u64) -> Self {
        self.anchors.push(AnchorSpec {
            kind: AnchorKind::Witness,
            checkpoint_position,
            tsa_time: None,
            malformed: false,
            unlinked: false,
            extra_raw: None,
        });
        self
    }

    pub fn with_anchor_token(mut self, checkpoint_position: u64, tsa_time: &str) -> Self {
        self.anchors.push(AnchorSpec {
            kind: AnchorKind::Token,
            checkpoint_position,
            tsa_time: Some(tsa_time.to_string()),
            malformed: false,
            unlinked: false,
            extra_raw: None,
        });
        self
    }

    /// Row-18 plumbing mutation: the anchor's own bytes are corrupted so
    /// they fail to parse.
    pub fn malformed_anchor(mut self, checkpoint_position: u64) -> Self {
        self.anchors.push(AnchorSpec {
            kind: AnchorKind::Token,
            checkpoint_position,
            tsa_time: None,
            malformed: true,
            unlinked: false,
            extra_raw: None,
        });
        self
    }

    /// Row-19 plumbing mutation: the anchor references a checkpoint
    /// position that is NOT in the bundle's checkpoint chain.
    pub fn unlinked_anchor(mut self, fake_checkpoint_position: u64) -> Self {
        self.anchors.push(AnchorSpec {
            kind: AnchorKind::Token,
            checkpoint_position: fake_checkpoint_position,
            tsa_time: None,
            malformed: false,
            unlinked: true,
            extra_raw: None,
        });
        self
    }

    /// Test-support: emit a witness anchor whose JSON carries `extra_raw`
    /// spliced verbatim before its closing brace. The ONLY way to inject a
    /// DUPLICATE key, since `serde_json::json!` collapses duplicates. The
    /// manifest hashes these exact bytes at build time, so the anchor reaches
    /// the verifier's parse (not a member-hash mismatch). Used to build the
    /// anchor duplicate-key / single-unknown-key Rust<->Python parity vectors.
    pub fn with_anchor_witness_raw_suffix(mut self, checkpoint_position: u64, extra_raw: &str) -> Self {
        self.anchors.push(AnchorSpec {
            kind: AnchorKind::Witness,
            checkpoint_position,
            tsa_time: None,
            malformed: false,
            unlinked: false,
            extra_raw: Some(extra_raw.as_bytes().to_vec()),
        });
        self
    }

    /// Test-support: like `with_anchor_witness_raw_suffix`, but the spliced
    /// suffix is RAW BYTES — so a non-UTF-8 byte (e.g. `0xFF`) can be injected
    /// inside an IGNORED anchor field's value, or an arbitrarily deep nesting can
    /// be spliced. Used to build the anchor bad-UTF-8 and deep-nesting
    /// Rust<->Python parity vectors. The manifest hashes these exact bytes at
    /// build time, so the anchor reaches the verifier's parse (not a member-hash
    /// mismatch), exercising the tolerant anchor parse on both sides.
    pub fn with_anchor_witness_raw_bytes_suffix(mut self, checkpoint_position: u64, extra_raw: &[u8]) -> Self {
        self.anchors.push(AnchorSpec {
            kind: AnchorKind::Witness,
            checkpoint_position,
            tsa_time: None,
            malformed: false,
            unlinked: false,
            extra_raw: Some(extra_raw.to_vec()),
        });
        self
    }

    // ---- per-record mutations ----

    /// Row-2 mutation: flip a byte in the signed record's payload, post-
    /// signing (leaves `sig` unchanged, so it no longer matches).
    pub fn tamper_record(mut self, position: u64) -> Self {
        self.record_mut(position).tamper = true;
        self
    }

    /// Row-4 mutation: declare a deliberately wrong `prev_head` for this
    /// record, signed honestly over the lie — the record's own signature
    /// stays valid (isolating this from row 2), but the declared link no
    /// longer matches the chain's actual running head.
    pub fn forge_prev_head(mut self, position: u64) -> Self {
        self.record_mut(position).forge_prev_head = true;
        self
    }

    /// Row-3 mutation: sign the record at `record_position` with the key
    /// that was authoritative at `use_key_at` instead of its own position
    /// — a genuinely valid signature, from a REAL chain key, just the
    /// wrong era.
    pub fn forge_position(mut self, record_position: u64, use_key_at: u64) -> Self {
        self.record_mut(record_position).sign_as_of_position = Some(use_key_at);
        self
    }

    /// Row-16 mutation: sign the record with a key that has never been
    /// part of this bundle's key chain.
    pub fn unknown_key_record(mut self, position: u64) -> Self {
        self.record_mut(position).sign_with_foreign_key = Some(key(250));
        self
    }

    /// Row-17 mutation: wrap the record in a 2-signature DSSE envelope
    /// (one signature valid) — the multi-signature wire form.
    pub fn multi_sig_record(mut self, position: u64) -> Self {
        self.record_mut(position).multi_sig = true;
        self
    }

    /// Row-22 mutation: sign a record-shaped payload under
    /// `PT_CHECKPOINT` instead of `PT_RECORD` — a context splice.
    pub fn wrong_payload_type_record(mut self, position: u64) -> Self {
        self.record_mut(position).wrong_payload_type = true;
        self
    }

    fn record_mut(&mut self, position: u64) -> &mut RecordSpec {
        self.records
            .iter_mut()
            .find(|r| r.position == position)
            .unwrap_or_else(|| panic!("no record at position {position}"))
    }

    // ---- structural / manifest mutations ----

    /// Row-10 mutation: the manifest's declared `counts.records` disagrees
    /// with the actual record count (the manifest is HONESTLY signed over
    /// this lie — its own signature stays valid; only the content is
    /// wrong).
    pub fn mismatched_manifest_counts(mut self) -> Self {
        self.mismatched_manifest_counts = true;
        self
    }

    /// Row-10 mutation: the manifest's declared `export_checkpoint`
    /// disagrees with the actual checkpoint chain.
    pub fn mismatched_manifest_export_checkpoint(mut self) -> Self {
        self.mismatched_manifest_export_checkpoint = true;
        self
    }

    /// Row-11 mutation: flip a byte in the manifest's signed payload,
    /// post-signing.
    pub fn tamper_manifest(mut self) -> Self {
        self.tamper_manifest = true;
        self
    }

    /// Row-12 mutation: flip a byte of a container member's bytes AFTER
    /// the manifest's declared hash was computed from the honest bytes —
    /// works for any member name, including `content/<position>`.
    pub fn tamper_member(mut self, name: &str) -> Self {
        self.tamper_member = Some(name.to_string());
        self
    }

    /// Row-12 convenience: tamper a specific record's content blob.
    pub fn tamper_content(self, position: u64) -> Self {
        self.tamper_member(&format!("content/{position}"))
    }

    /// Row-21 mutation: an extra, otherwise-unused whitelisted member is
    /// physically present in the container but never listed in the
    /// manifest's `members` map.
    pub fn unlisted_container_member(mut self) -> Self {
        self.unlisted_container_member = true;
        self
    }

    /// Row-21 mutation: the manifest declares a hash for `name` but it is
    /// never physically added to the container.
    pub fn missing_container_member(mut self, name: &str) -> Self {
        self.missing_container_member = Some(name.to_string());
        self
    }

    /// Row-21 mutation: `name` appears TWICE in the raw container (hand-
    /// crafted ZIP bytes — `ContainerWriter` refuses this by construction,
    /// so this bypasses it to prove the reader independently rejects it).
    pub fn duplicate_member(mut self, name: &str) -> Self {
        self.duplicate_member = Some(name.to_string());
        self
    }

    /// Row-25: an explicit zero-record range export (also true by default
    /// if [`Self::add_record`] is never called — this method exists so a
    /// test can name the intent).
    pub fn zero_records(mut self) -> Self {
        self.zero_records = true;
        self
    }

    /// Row-24 mutation: the end checkpoint's `at` is set earlier than the
    /// start checkpoint's — a wall-clock non-monotonicity with no chain-
    /// position contradiction (VALID + WARNING, never TAMPERED).
    pub fn backwards_checkpoint_clock(mut self) -> Self {
        self.backwards_checkpoint_clock = true;
        self
    }

    /// F2 mutation: appends one extra, honestly-signed, IN-RANGE erasure
    /// record whose `target` does not resolve to any record actually in
    /// the bundle — a dangling erasure that must never be silently
    /// accepted.
    pub fn dangling_erasure_target(mut self, target: u64) -> Self {
        self.dangling_erasure_target = Some(target);
        self
    }

    /// F2 mutation, closure variant: appends one extra erasure record to
    /// `journal/closure.jsonl` whose `target` does not resolve to any
    /// in-range record.
    pub fn dangling_closure_erasure_target(mut self, target: u64) -> Self {
        self.dangling_closure_erasure_target = Some(target);
        self
    }

    /// Structural-violation mutation: overrides the (single)
    /// `erase_via_closure` record's declared `position` field so it falls
    /// INSIDE the exported range instead of strictly outside it — closure
    /// records claiming an in-range position are structurally invalid
    /// (they should have been ordinary epoch entries).
    pub fn closure_record_position_override(mut self, position: u64) -> Self {
        self.closure_record_position_override = Some(position);
        self
    }

    /// Row-2-in-closure mutation (test-plan case B4): flips a byte in the
    /// (single) closure erasure record's signed payload post-signing —
    /// closure records go through the SAME signature authentication as any
    /// other record, never a rubber stamp.
    pub fn tamper_closure_record(mut self) -> Self {
        self.tamper_closure_record = true;
        self
    }

    /// Row-22-in-closure mutation: the (single) closure erasure record
    /// declares `kind: "note"` instead of `kind: "erasure"`.
    pub fn closure_record_wrong_kind(mut self) -> Self {
        self.closure_record_wrong_kind = true;
        self
    }

    /// Closure-count cross-check mutation: the manifest's declared
    /// `counts.closure` disagrees with the actual number of
    /// `journal/closure.jsonl` entries (the manifest is honestly signed
    /// over this lie, mirroring `mismatched_manifest_counts`).
    pub fn mismatched_manifest_closure_count(mut self) -> Self {
        self.mismatched_manifest_closure_count = true;
        self
    }

    /// Windowed-export recipe: the exported range starts AFTER `position`
    /// (a real, honest checkpoint boundary this builder creates) instead of
    /// at genesis — a compliance re-export of a recent window over a
    /// journal with more history before it. `position` must be `<=` the
    /// total record count (real records + any in-range erasures); records
    /// and in-range erasures at or before `position` still exist in the
    /// FULL chain (their bytes are hashed into the window's start head) but
    /// are excluded from the exported `journal/epoch-0.jsonl`.
    pub fn export_window_start(mut self, position: u64) -> Self {
        self.window_start = Some(position);
        self
    }

    /// Task-16 timestamp-grammar hook: set the SIGNED `at` strings of the
    /// start and end checkpoints verbatim (see [`Self::checkpoint_times`]).
    /// Overrides both the default times and `backwards_checkpoint_clock`.
    pub fn with_checkpoint_times(mut self, start_at: &str, end_at: &str) -> Self {
        self.checkpoint_times = Some((start_at.to_string(), end_at.to_string()));
        self
    }

    // ---- build ----

    /// The key valid at `position` per the HONEST recipe (mirrors
    /// `key_at_position`'s half-open-interval rule locally, since the
    /// builder needs this before any `KeyChain` exists to ask).
    fn signer_for_position(&self, position: u64) -> &SigningKey {
        self.rotations
            .iter()
            .rev()
            .find(|(p, _)| *p <= position)
            .map(|(_, sk)| sk)
            .unwrap_or(&self.genesis)
    }

    pub fn build(&self) -> Vec<u8> {
        // ---- 1. key chain ----
        let mut key_lines: Vec<Vec<u8>> = Vec::new();
        let genesis_payload = serde_json::json!({
            "kind": "genesis",
            "position": 0,
            "signing_key": hex32(&self.genesis.verifying_key()),
            "recovery_key": self.recovery.as_ref().map(|r| hex32(&r.verifying_key())),
            "predecessor_genesis": null,
            "key_prev": hex::encode(GENESIS_PREV),
        });
        let genesis_bytes = serde_json::to_vec(&genesis_payload).expect("static json");
        let genesis_env = sign_envelope(
            &self.genesis,
            &hex32(&self.genesis.verifying_key()),
            PT_KEYRECORD,
            &genesis_bytes,
        );
        key_lines.push(genesis_env.to_line());
        let mut key_chain_head = head_hash(&GENESIS_PREV, &leaf_hash(key_lines.last().unwrap()));
        let mut predecessor: &SigningKey = &self.genesis;

        for (position, new_key) in &self.rotations {
            let payload = serde_json::json!({
                "kind": "rotation",
                "position": position,
                "new_signing_key": hex32(&new_key.verifying_key()),
                "key_prev": hex::encode(key_chain_head),
            });
            let payload_bytes = serde_json::to_vec(&payload).expect("static json");
            let actual_signer: SigningKey = if self.corrupt_rotation_signer_at == Some(*position) {
                key(66) // an impostor, never the true predecessor
            } else {
                predecessor.clone()
            };
            let env = sign_envelope(
                &actual_signer,
                &hex32(&predecessor.verifying_key()),
                PT_KEYRECORD,
                &payload_bytes,
            );
            key_lines.push(env.to_line());
            key_chain_head = head_hash(&key_chain_head, &leaf_hash(key_lines.last().unwrap()));
            predecessor = new_key;
        }

        if let Some((position, claimed_time)) = &self.compromise {
            let compromised_signer = self.signer_for_position(position.saturating_sub(1));
            let payload = serde_json::json!({
                "kind": "compromise",
                "position": position,
                "compromised_key": hex32(&compromised_signer.verifying_key()),
                "claimed_compromise_time": claimed_time,
                "key_prev": hex::encode(key_chain_head),
            });
            let payload_bytes = serde_json::to_vec(&payload).expect("static json");
            let recovery = self
                .recovery
                .as_ref()
                .expect("with_compromise requires with_recovery_key");
            let actual_signer: SigningKey = if self.corrupt_compromise_signer {
                key(67) // not the recovery key
            } else {
                recovery.clone()
            };
            let env = sign_envelope(
                &actual_signer,
                &hex32(&recovery.verifying_key()),
                PT_KEYRECORD,
                &payload_bytes,
            );
            key_lines.push(env.to_line());
            key_chain_head = head_hash(&key_chain_head, &leaf_hash(key_lines.last().unwrap()));
        }
        let _ = key_chain_head; // not needed past this point

        // ---- 2. real records (positions 1..=N) ----
        let mut journal_lines: Vec<Vec<u8>> = Vec::new();
        let mut running_head = GENESIS_PREV;
        // Parallel to `journal_lines`: `heads_after[i]` is the running head
        // AFTER `journal_lines[i]` (position `i + 1`) — snapshotted so
        // `export_window_start` can look up the correct head for an
        // arbitrary mid-journal checkpoint boundary without re-walking.
        let mut heads_after: Vec<[u8; 32]> = Vec::new();

        for rec in &self.records {
            let content_hash_hex = rec.content.as_ref().map(|c| {
                let salt = derived_32("content-salt", rec.position);
                hex::encode(content_hash(&salt, c))
            });
            let declared_prev_head = if rec.forge_prev_head {
                let mut bogus = running_head;
                bogus[0] ^= 0xFF; // deliberately wrong, but still 32 well-formed bytes
                bogus
            } else {
                running_head
            };
            let payload_bytes = if let Some(raw_body) = &rec.raw_body {
                // Hand-assemble the payload so the RAW `body` bytes survive
                // verbatim into what gets signed and member-hashed (a dup key /
                // out-of-range number / deep nest that `serde_json::json!`
                // could never emit). Every OTHER field stays honest, so the
                // ONLY thing a verifier can find is the malformed body.
                let mut p: Vec<u8> = Vec::new();
                p.extend_from_slice(
                    format!(
                        "{{\"position\":{},\"prev_head\":\"{}\",\"class\":{},\"kind\":{},\"at\":{},\"actor\":{{\"id\":\"builder\"}},",
                        rec.position,
                        hex::encode(declared_prev_head),
                        serde_json::to_string(&rec.class).expect("string"),
                        serde_json::to_string(&rec.kind).expect("string"),
                        serde_json::to_string(&rec.at).expect("string"),
                    )
                    .as_bytes(),
                );
                match &content_hash_hex {
                    Some(h) => p.extend_from_slice(format!("\"content_hash\":\"{h}\",").as_bytes()),
                    None => p.extend_from_slice(b"\"content_hash\":null,"),
                }
                p.extend_from_slice(b"\"body\":");
                p.extend_from_slice(raw_body);
                p.extend_from_slice(b",\"backfilled\":false}");
                p
            } else {
                let payload = serde_json::json!({
                    "position": rec.position,
                    "prev_head": hex::encode(declared_prev_head),
                    "class": rec.class,
                    "kind": rec.kind,
                    "at": rec.at,
                    "actor": {"id": "builder"},
                    "content_hash": content_hash_hex,
                    "body": {"seq": rec.position},
                    "backfilled": false,
                });
                serde_json::to_vec(&payload).expect("static json")
            };
            let payload_type = if rec.wrong_payload_type {
                PT_CHECKPOINT
            } else {
                PT_RECORD
            };

            let (signer, declared_keyid): (SigningKey, String) =
                if let Some(foreign) = &rec.sign_with_foreign_key {
                    (foreign.clone(), hex32(&foreign.verifying_key()))
                } else if let Some(as_of) = rec.sign_as_of_position {
                    let sk = self.signer_for_position(as_of).clone();
                    let vk = sk.verifying_key();
                    (sk, hex32(&vk))
                } else {
                    let sk = self.signer_for_position(rec.position).clone();
                    let vk = sk.verifying_key();
                    (sk, hex32(&vk))
                };

            let mut env = sign_envelope(&signer, &declared_keyid, payload_type, &payload_bytes);
            // Captured BEFORE any post-hoc tamper below — this is what a
            // real exporter would have honestly written and what the NEXT
            // record's `prev_head` must continue to declare, exactly
            // reproducing a real attacker editing an already-exported
            // bundle (test-plan case 1.1's scenario) rather than the
            // exporter itself having signed corrupted content.
            let honest_leaf = leaf_hash(&env.to_line());

            if rec.tamper {
                // Row 2: flip a byte in the free-form `class` string,
                // leaving `sig` stale against the new payload bytes. This
                // happens AFTER `honest_leaf` was captured, so it does NOT
                // retroactively change what subsequent records expect.
                flip_last_byte_of(&mut env.payload, rec.class.as_bytes());
            }

            let line = if rec.multi_sig {
                // Row 17: hand-build the multi-signature wire form (the
                // `Envelope` struct cannot represent it by construction).
                let multi = serde_json::json!({
                    "payloadType": env.payload_type,
                    "payload": STANDARD.encode(&env.payload),
                    "signatures": [
                        {"sig": STANDARD.encode(env.sig), "keyid": env.keyid},
                        {"sig": STANDARD.encode(env.sig), "keyid": "second-sig-keyid"},
                    ],
                });
                serde_json::to_vec(&multi).expect("static json")
            } else {
                env.to_line()
            };

            // Bookkeeping for the NEXT record's `prev_head`: every
            // mutation OTHER than `tamper` (multi-sig wrapping, a wrong
            // signer/payloadType) is a genuine construction-time choice —
            // what actually got signed and written — so the chain must
            // continue from those ACTUAL bytes to keep those mutations
            // correctly isolated to their own rows (3/16/17/22) without an
            // incidental row-4 finding on the following record.
            let leaf_for_bookkeeping = if rec.tamper { honest_leaf } else { leaf_hash(&line) };
            running_head = head_hash(&running_head, &leaf_for_bookkeeping);
            journal_lines.push(line);
            heads_after.push(running_head);
        }

        // `export_window_start`: computed early (needed by the content
        // filter immediately below) — `range_start_head`, which DOES depend
        // on `heads_after` being fully populated, is looked up later,
        // immediately before the checkpoints are built.
        let range_start = self.window_start.unwrap_or(0);

        // ---- 3. content blobs + erasure records (positions N+1..N+E) ----
        let mut content_members: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut withheld_erased_count: u64 = 0;
        let mut erasure_target_positions: Vec<(u64, ErasureMode)> = self
            .records
            .iter()
            .filter_map(|r| r.content.as_ref().map(|_| r.position))
            .filter_map(|p| self.erase_positions.get(&p).map(|m| (p, *m)))
            .collect();
        erasure_target_positions.sort_by_key(|(p, _)| *p);

        for rec in &self.records {
            let Some(content) = &rec.content else { continue };
            if rec.position <= range_start {
                // Outside this export's own window (see
                // `export_window_start`) — a real exporter never even
                // fetches content for a position it isn't exporting.
                continue;
            }
            let salt = derived_32("content-salt", rec.position);
            let mut blob = Vec::with_capacity(32 + content.len());
            blob.extend_from_slice(&salt);
            blob.extend_from_slice(content);
            match self.erase_positions.get(&rec.position) {
                None | Some(ErasureMode::KeepContentDespiteErasure) => {
                    content_members.insert(format!("content/{}", rec.position), blob);
                }
                Some(ErasureMode::Honest)
                | Some(ErasureMode::DropErasureRecord)
                | Some(ErasureMode::HonestClosure) => {
                    // content omitted
                }
            }
        }

        // Pass 1: in-range erasures (Honest/KeepContentDespiteErasure) are
        // ordinary later journal entries, appended to `journal_lines` —
        // unchanged from before `erase_via_closure` existed. `HonestClosure`
        // entries are deliberately skipped here; pass 2 below emits them
        // into a SEPARATE `closure_lines` vector instead, which is what
        // keeps them out of `total_n`/the exported range boundary.
        for (target_position, mode) in &erasure_target_positions {
            if *mode == ErasureMode::DropErasureRecord || *mode == ErasureMode::HonestClosure {
                continue;
            }
            // The next sequential position after everything already in
            // `journal_lines` (the N real records plus any erasure records
            // already appended this loop).
            let position = journal_lines.len() as u64 + 1;
            let payload = serde_json::json!({
                "position": position,
                "prev_head": hex::encode(running_head),
                "class": "evidence-record",
                "kind": "erasure",
                "at": "2026-01-01T02:00:00Z",
                "actor": {"id": "builder"},
                "content_hash": null,
                "body": {"target": target_position},
                "backfilled": false,
            });
            let payload_bytes = serde_json::to_vec(&payload).expect("static json");
            let signer = self.signer_for_position(position);
            let env = sign_envelope(signer, &hex32(&signer.verifying_key()), PT_RECORD, &payload_bytes);
            let line = env.to_line();
            running_head = head_hash(&running_head, &leaf_hash(&line));
            journal_lines.push(line);
            heads_after.push(running_head);
            // "withheld_erased" counts records whose content is ACTUALLY
            // absent because of erasure (row 13's definition) — NOT every
            // position with an erasure record. `KeepContentDespiteErasure`
            // deliberately leaves the content physically present (that IS
            // the row-15 mutation), so it must not be counted as withheld.
            // `*target_position > range_start` (fix round 1, 2026-08-25):
            // a target OUTSIDE this export's own window isn't part of this
            // bundle's content at all — nothing here to mark withheld —
            // otherwise the manifest's own declared count would disagree
            // with what the verifier independently observes (a spurious
            // row-10 finding on top of the intended row-13/15 one).
            if *mode == ErasureMode::Honest && *target_position > range_start {
                withheld_erased_count += 1;
            }
        }

        // Optional dangling in-range erasure (F2): an honestly-signed
        // erasure record whose target matches no record anywhere in the
        // bundle.
        if let Some(target) = self.dangling_erasure_target {
            let position = journal_lines.len() as u64 + 1;
            let payload = serde_json::json!({
                "position": position,
                "prev_head": hex::encode(running_head),
                "class": "evidence-record",
                "kind": "erasure",
                "at": "2026-01-01T02:00:00Z",
                "actor": {"id": "builder"},
                "content_hash": null,
                "body": {"target": target},
                "backfilled": false,
            });
            let payload_bytes = serde_json::to_vec(&payload).expect("static json");
            let signer = self.signer_for_position(position);
            let env = sign_envelope(signer, &hex32(&signer.verifying_key()), PT_RECORD, &payload_bytes);
            let line = env.to_line();
            running_head = head_hash(&running_head, &leaf_hash(&line));
            journal_lines.push(line);
            heads_after.push(running_head);
        }

        let total_n = journal_lines.len() as u64;

        // Pass 2: `journal/closure.jsonl` entries. Positions continue the
        // SAME global sequence (as if these erasures really did happen
        // later, past this export's own range boundary), but they are
        // deliberately NOT counted in `total_n`/`effective_end` above — that
        // is the entire mechanism that places them "outside the exported
        // range" for the verifier's closure interpretation to exercise.
        // Their `prev_head`/chain linkage is cosmetic only (closure records
        // are envelope-authenticated, never chain-walked — see verify.rs)
        // but computed honestly for realism.
        let mut closure_lines: Vec<Vec<u8>> = Vec::new();
        let mut closure_head = running_head;
        for (target_position, mode) in &erasure_target_positions {
            if *mode != ErasureMode::HonestClosure {
                continue;
            }
            let natural_position = total_n + closure_lines.len() as u64 + 1;
            let position = self.closure_record_position_override.unwrap_or(natural_position);
            let kind = if self.closure_record_wrong_kind { "note" } else { "erasure" };
            let payload = serde_json::json!({
                "position": position,
                "prev_head": hex::encode(closure_head),
                "class": "evidence-record",
                "kind": kind,
                "at": "2026-01-01T03:00:00Z",
                "actor": {"id": "builder"},
                "content_hash": null,
                "body": {"target": target_position},
                "backfilled": false,
            });
            let payload_bytes = serde_json::to_vec(&payload).expect("static json");
            let signer = self.signer_for_position(position);
            let mut env = sign_envelope(signer, &hex32(&signer.verifying_key()), PT_RECORD, &payload_bytes);
            if self.tamper_closure_record {
                flip_last_byte_of(&mut env.payload, b"evidence-record");
            }
            let line = env.to_line();
            closure_head = head_hash(&closure_head, &leaf_hash(&line));
            closure_lines.push(line);
            // Same `> range_start` guard as the in-range loop above — a
            // closure erasure's target must actually be in THIS export's
            // window to count as withheld here.
            if *target_position > range_start {
                withheld_erased_count += 1;
            }
        }
        if let Some(target) = self.dangling_closure_erasure_target {
            let position = total_n + closure_lines.len() as u64 + 1;
            let payload = serde_json::json!({
                "position": position,
                "prev_head": hex::encode(closure_head),
                "class": "evidence-record",
                "kind": "erasure",
                "at": "2026-01-01T03:00:00Z",
                "actor": {"id": "builder"},
                "content_hash": null,
                "body": {"target": target},
                "backfilled": false,
            });
            let payload_bytes = serde_json::to_vec(&payload).expect("static json");
            let signer = self.signer_for_position(position);
            let env = sign_envelope(signer, &hex32(&signer.verifying_key()), PT_RECORD, &payload_bytes);
            let line = env.to_line();
            closure_head = head_hash(&closure_head, &leaf_hash(&line));
            closure_lines.push(line);
        }
        let _ = closure_head; // not read past this point

        // ---- 4. checkpoints ----
        let effective_end = if self.zero_records { 0 } else { total_n };
        let effective_head = if self.zero_records { GENESIS_PREV } else { running_head };

        // `range_start_head`: looked up from the `heads_after` snapshots
        // taken above, never re-walked.
        let range_start_head = if range_start == 0 {
            GENESIS_PREV
        } else {
            heads_after[(range_start - 1) as usize]
        };

        // Task-16 override wins over both the hardcoded times and
        // `backwards_checkpoint_clock` when present.
        let start_at: &str = match &self.checkpoint_times {
            Some((s, _)) => s.as_str(),
            None => "2026-01-01T00:00:00Z",
        };
        let start_signer = self.signer_for_position(range_start);
        let start_payload = serde_json::json!({
            "position": range_start,
            "head": hex::encode(range_start_head),
            "count": range_start,
            "at": start_at,
            "keyid": hex32(&start_signer.verifying_key()),
            "cp_prev": hex::encode(GENESIS_PREV),
        });
        let start_payload_bytes = serde_json::to_vec(&start_payload).expect("static json");
        let start_env = sign_envelope(
            start_signer,
            &hex32(&start_signer.verifying_key()),
            PT_CHECKPOINT,
            &start_payload_bytes,
        );
        let start_line = start_env.to_line();

        let checkpoint_lines: Vec<Vec<u8>> = if effective_end == 0 {
            // A genesis-equivalent, single-checkpoint bundle (row 25) — one
            // checkpoint is both the start and end boundary.
            vec![start_line]
        } else {
            let checkpoint_chain_head_after_start = head_hash(&GENESIS_PREV, &leaf_hash(&start_line));
            let end_signer = self.signer_for_position(effective_end);
            let end_at: &str = if let Some((_, e)) = &self.checkpoint_times {
                e.as_str()
            } else if self.backwards_checkpoint_clock {
                "2025-01-01T00:00:00Z" // earlier than the start checkpoint's "2026-01-01T00:00:00Z"
            } else {
                "2026-01-01T01:00:00Z"
            };
            let end_payload = serde_json::json!({
                "position": effective_end,
                "head": hex::encode(effective_head),
                "count": effective_end,
                "at": end_at,
                "keyid": hex32(&end_signer.verifying_key()),
                "cp_prev": hex::encode(checkpoint_chain_head_after_start),
            });
            let end_payload_bytes = serde_json::to_vec(&end_payload).expect("static json");
            let end_env = sign_envelope(
                end_signer,
                &hex32(&end_signer.verifying_key()),
                PT_CHECKPOINT,
                &end_payload_bytes,
            );
            vec![start_line, end_env.to_line()]
        };

        // ---- 5. anchors ----
        let mut anchor_members: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for (i, a) in self.anchors.iter().enumerate() {
            let bytes = if a.malformed {
                b"{not valid json".to_vec()
            } else {
                let kind_str = match a.kind {
                    AnchorKind::Witness => "witness",
                    AnchorKind::Token => "token",
                };
                let json = serde_json::json!({
                    "kind": kind_str,
                    "checkpoint_position": a.checkpoint_position,
                    "tsa_time": a.tsa_time,
                    "unlinked_marker": a.unlinked,
                });
                let bytes = serde_json::to_vec(&json).expect("static json");
                match &a.extra_raw {
                    None => bytes,
                    Some(extra) => {
                        // Splice `,{extra}` before the object's closing brace
                        // (the ONLY way to emit a duplicate key). `rposition`
                        // on `to_vec` output always finds the trailing `}`.
                        let cut = bytes
                            .iter()
                            .rposition(|&b| b == b'}')
                            .expect("serialized object always ends in a closing brace");
                        let mut spliced = bytes[..cut].to_vec();
                        spliced.push(b',');
                        spliced.extend_from_slice(extra);
                        spliced.extend_from_slice(&bytes[cut..]);
                        spliced
                    }
                }
            };
            anchor_members.insert(format!("anchors/a{i}.json"), bytes);
        }

        // ---- 6. assemble container members ----
        // Windowed export: only lines with position > range_start are
        // actually exported — `journal_lines[range_start..]`, since index
        // `i` holds position `i + 1` (see the `heads_after` comment above).
        let epoch_bytes = join_lines(&journal_lines[range_start as usize..]);
        let checkpoints_bytes = join_lines(&checkpoint_lines);
        let keys_bytes = join_lines(&key_lines);

        let mut members: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        members.insert("journal/epoch-0.jsonl".to_string(), epoch_bytes);
        members.insert("checkpoints.jsonl".to_string(), checkpoints_bytes);
        members.insert("trust/keys.jsonl".to_string(), keys_bytes);
        // Always present (empty when there is nothing to carry forward) —
        // matches the real exporter's container shape.
        members.insert("journal/closure.jsonl".to_string(), join_lines(&closure_lines));
        for (name, bytes) in content_members {
            members.insert(name, bytes);
        }
        for (name, bytes) in anchor_members {
            members.insert(name, bytes);
        }
        if self.unlisted_container_member {
            members.insert("derived/extra.bin".to_string(), b"unlisted".to_vec());
        }

        // ---- 7. manifest (honest values, then row-10 lies if requested) ----
        // `windowed_n`: the actual number of records THIS export carries —
        // `total_n` (every real+erasure entry) minus whatever `range_start`
        // excludes. Equal to `total_n` when `window_start` is unset (the
        // overwhelmingly common case, and every pre-existing test).
        let windowed_n = total_n - range_start;
        let manifest_records_count = if self.mismatched_manifest_counts {
            windowed_n + 1 // a lie: claims one more than reality
        } else {
            windowed_n
        };
        let manifest_end_position = if self.mismatched_manifest_export_checkpoint {
            effective_end + 1 // a lie: a position the checkpoint chain
                               // doesn't actually have
        } else {
            effective_end
        };
        let manifest_end_head = if self.mismatched_manifest_export_checkpoint {
            let mut h = effective_head;
            h[0] ^= 0xFF; // a lie
            h
        } else {
            effective_head
        };
        let manifest_closure_count = if self.mismatched_manifest_closure_count {
            closure_lines.len() as u64 + 1 // a lie: claims one more than reality
        } else {
            closure_lines.len() as u64
        };

        let mut manifest_members: BTreeMap<String, String> = BTreeMap::new();
        for (name, bytes) in &members {
            manifest_members.insert(name.clone(), hex::encode(Sha256::digest(bytes)));
        }
        if let Some(missing_name) = &self.missing_container_member {
            // Declared in the manifest, but never physically added below.
            manifest_members.insert(missing_name.clone(), hex::encode(Sha256::digest(b"phantom")));
        }
        if self.unlisted_container_member {
            // Physically present (added above) but deliberately NOT
            // declared here.
            manifest_members.remove("derived/extra.bin");
        }

        let manifest_signer = self.signer_for_position(manifest_end_position);
        let manifest_payload = serde_json::json!({
            "scope": {"range": [range_start, effective_end], "classes": ["evidence-record"], "spaces": null},
            "counts": {
                "records": manifest_records_count,
                "closure": manifest_closure_count,
                "withheld_erased": withheld_erased_count,
            },
            "export_checkpoint": {
                "position": manifest_end_position,
                "head": hex::encode(manifest_end_head),
                "count": effective_end,
            },
            "tool": {"exporter": "docbrain-test/0.0.0"},
            "members": manifest_members,
        });
        let manifest_payload_bytes = serde_json::to_vec(&manifest_payload).expect("static json");
        let mut manifest_env = sign_envelope(
            manifest_signer,
            &hex32(&manifest_signer.verifying_key()),
            PT_MANIFEST,
            &manifest_payload_bytes,
        );
        if self.tamper_manifest {
            flip_last_byte_of(&mut manifest_env.payload, b"docbrain-test/0.0.0");
        }
        members.insert(MANIFEST_MEMBER_NAME.to_string(), manifest_env.to_line());

        // ---- 8. row-12 member tamper (post-manifest-hash corruption) ----
        if let Some(name) = &self.tamper_member
            && let Some(bytes) = members.get_mut(name)
        {
            if !bytes.is_empty() {
                let last = bytes.len() - 1;
                bytes[last] ^= 0x01;
            } else {
                bytes.push(0x01);
            }
        }

        // ---- 9. serialize ----
        if let Some(dup_name) = &self.duplicate_member {
            return build_raw_zip_with_duplicate(&members, dup_name);
        }

        let mut writer = ContainerWriter::new();
        for (name, bytes) in &members {
            if self.missing_container_member.as_deref() == Some(name.as_str()) {
                continue; // declared in manifest, never physically added
            }
            writer
                .add_member(name, bytes.clone())
                .expect("builder only ever emits whitelisted, unique names");
        }
        writer.finish().expect("builder never exceeds writer limits")
    }
}

/// Builds raw STORE-only ZIP bytes with `dup_name` appearing TWICE (bypasses
/// `ContainerWriter::add_member`'s own duplicate-name rejection) — the
/// row-21 `duplicate_member` mutation. Uses the exact same field values
/// `ContainerWriter::finish` emits (via the now-`pub(crate)` constants in
/// `container.rs`), so this is a minimal, format-faithful hand roll, not an
/// independent reimplementation that could drift.
fn build_raw_zip_with_duplicate(members: &BTreeMap<String, Vec<u8>>, dup_name: &str) -> Vec<u8> {
    let mut entries: Vec<(String, Vec<u8>)> =
        members.iter().map(|(n, b)| (n.clone(), b.clone())).collect();
    if let Some((_, bytes)) = entries.iter().find(|(n, _)| n == dup_name).cloned() {
        entries.push((dup_name.to_string(), bytes));
    }

    let mut out = Vec::new();
    let mut local_offsets = Vec::with_capacity(entries.len());
    for (name, data) in &entries {
        let offset = out.len() as u32;
        local_offsets.push(offset);
        let name_bytes = name.as_bytes();
        let size = data.len() as u32;
        let crc = crate::container::crc32(data);

        out.extend_from_slice(&LFH_SIG);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&GPBF_UTF8.to_le_bytes());
        out.extend_from_slice(&METHOD_STORE.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);
    }

    let cd_start = out.len() as u32;
    for ((name, data), &local_offset) in entries.iter().zip(&local_offsets) {
        let name_bytes = name.as_bytes();
        let size = data.len() as u32;
        let crc = crate::container::crc32(data);

        out.extend_from_slice(&CDFH_SIG);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&GPBF_UTF8.to_le_bytes());
        out.extend_from_slice(&METHOD_STORE.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&local_offset.to_le_bytes());
        out.extend_from_slice(name_bytes);
    }
    let cd_end = out.len() as u32;
    let cd_size = cd_end - cd_start;
    let entry_count = entries.len() as u16;

    out.extend_from_slice(&EOCD_SIG);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&entry_count.to_le_bytes());
    out.extend_from_slice(&entry_count.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::{range_bounds, verify_checkpoint_chain};
    use crate::chain::walk_chain;
    use crate::container::ContainerReader;
    use crate::keys::verify_key_chain;
    use crate::manifest::{verify_manifest, verify_members};

    /// Round-trips the HONEST (no mutations) bundle through every Task
    /// 1-6 primitive directly — isolates bugs in the builder's own
    /// construction from bugs in the (not-yet-written) verify.rs pipeline.
    /// If this fails, the builder is wrong, not the pipeline.
    #[test]
    fn honest_bundle_round_trips_through_every_primitive() {
        let bytes = BundleBuilder::new().add_records(5).build();
        let reader = ContainerReader::open(&bytes).expect("container must open");
        let key_lines: Vec<Vec<u8>> = split_lines(reader.member_bytes("trust/keys.jsonl").unwrap());
        let key_refs: Vec<&[u8]> = key_lines.iter().map(|l| l.as_slice()).collect();
        let keys = verify_key_chain(&key_refs).expect("key chain must verify");
        let manifest = verify_manifest(&reader, &keys).expect("manifest must verify");
        verify_members(&reader, &manifest).expect("member hashes must verify");
        let cp_lines: Vec<Vec<u8>> = split_lines(reader.member_bytes("checkpoints.jsonl").unwrap());
        let cp_refs: Vec<&[u8]> = cp_lines.iter().map(|l| l.as_slice()).collect();
        let checkpoints = verify_checkpoint_chain(&cp_refs, &keys).expect("checkpoints must verify");
        let bounds = range_bounds(&checkpoints, manifest.scope().range()).expect("range must resolve");
        let epoch_lines: Vec<Vec<u8>> = split_lines(reader.member_bytes("journal/epoch-0.jsonl").unwrap());
        let epoch_refs: Vec<&[u8]> = epoch_lines.iter().map(|l| l.as_slice()).collect();
        let (final_pos, final_head) =
            walk_chain(bounds.start_position, bounds.start_head, &epoch_refs).expect("chain must walk");
        assert_eq!(final_pos, bounds.end_position);
        assert_eq!(final_head, bounds.end_head);
        assert_eq!(manifest.counts().records(), 5);
        assert_eq!(epoch_lines.len(), 5);
    }

    #[test]
    fn honest_bundle_with_rotation_and_content_and_erasure_round_trips() {
        let bytes = BundleBuilder::new()
            .with_rotation(3)
            .add_records(5)
            .with_content(1, b"hello world")
            .with_content(2, b"erase me")
            .erase(2)
            .build();
        let reader = ContainerReader::open(&bytes).expect("container must open");
        let key_lines: Vec<Vec<u8>> = split_lines(reader.member_bytes("trust/keys.jsonl").unwrap());
        let key_refs: Vec<&[u8]> = key_lines.iter().map(|l| l.as_slice()).collect();
        let keys = verify_key_chain(&key_refs).expect("key chain must verify");
        let manifest = verify_manifest(&reader, &keys).expect("manifest must verify");
        verify_members(&reader, &manifest).expect("member hashes must verify");
        let cp_lines: Vec<Vec<u8>> = split_lines(reader.member_bytes("checkpoints.jsonl").unwrap());
        let cp_refs: Vec<&[u8]> = cp_lines.iter().map(|l| l.as_slice()).collect();
        let checkpoints = verify_checkpoint_chain(&cp_refs, &keys).expect("checkpoints must verify");
        let bounds = range_bounds(&checkpoints, manifest.scope().range()).expect("range must resolve");
        let epoch_lines: Vec<Vec<u8>> = split_lines(reader.member_bytes("journal/epoch-0.jsonl").unwrap());
        let epoch_refs: Vec<&[u8]> = epoch_lines.iter().map(|l| l.as_slice()).collect();
        let (final_pos, final_head) =
            walk_chain(bounds.start_position, bounds.start_head, &epoch_refs).expect("chain must walk");
        assert_eq!(final_pos, bounds.end_position);
        assert_eq!(final_head, bounds.end_head);
        // 5 real records + 1 erasure record = 6 journal entries.
        assert_eq!(epoch_lines.len(), 6);
        assert_eq!(manifest.counts().withheld_erased(), 1);
        assert!(reader.member_bytes("content/1").is_ok());
        assert!(reader.member_bytes("content/2").is_err(), "erased content must be absent");
    }

    #[test]
    fn honest_zero_record_bundle_round_trips() {
        let bytes = BundleBuilder::new().build(); // no records added
        let reader = ContainerReader::open(&bytes).expect("container must open");
        let key_lines: Vec<Vec<u8>> = split_lines(reader.member_bytes("trust/keys.jsonl").unwrap());
        let key_refs: Vec<&[u8]> = key_lines.iter().map(|l| l.as_slice()).collect();
        let keys = verify_key_chain(&key_refs).expect("key chain must verify");
        let manifest = verify_manifest(&reader, &keys).expect("manifest must verify");
        verify_members(&reader, &manifest).expect("member hashes must verify");
        assert_eq!(manifest.counts().records(), 0);
        let cp_lines: Vec<Vec<u8>> = split_lines(reader.member_bytes("checkpoints.jsonl").unwrap());
        assert_eq!(cp_lines.len(), 1, "genesis-equivalent bundle has one checkpoint");
    }

    fn split_lines(bytes: &[u8]) -> Vec<Vec<u8>> {
        bytes
            .split(|&b| b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| l.to_vec())
            .collect()
    }
}
