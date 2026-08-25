// SPDX-License-Identifier: MIT
//! The verification pipeline (spec "Verifier" section): the offline,
//! deterministic verdict engine that assembles every primitive from
//! Tasks 1-6 into the actual `.dbev` verifier.
//!
//! ## Pipeline order (two phases)
//!
//! **Phase A — sequential, fail-fast bootstrap.** Container profile → key
//! chain from genesis → manifest envelope (needs the key chain to resolve
//! its signer) → member hashes vs manifest → checkpoint chain from genesis
//! → range boundaries. Each step is a hard data dependency for the next
//! (`verify_manifest` literally takes `&KeyChain`; `verify_checkpoint_chain`
//! and `range_bounds` likewise), so ANY Phase A failure returns immediately
//! with exactly one [`Finding`] — nothing past that point is safe to
//! interpret (design doc: "container/manifest byte-authentication failures
//! dominate; nothing else is interpreted"). Folding the key-chain and
//! member-hash steps into this same fail-fast tier (rather than only
//! container+manifest, as the design doc's prose literally lists) is this
//! module's own resolution of a genuine ordering ambiguity: `verify_manifest`
//! cannot run without an already-verified `KeyChain`, so "manifest envelope
//! (DSSE from exact bytes)" is structurally unreachable before "key chain
//! from genesis" despite the prose listing them in the other order. See
//! `task-7-report.md` for the full reasoning.
//!
//! **Phase B — findings accumulate.** Per-record signature/keyid/
//! payloadType (rows 2,3,16,17); chain-link recomputation (row 4);
//! scope/count cross-checks (row 10); compromise classification (rows
//! 7,8,9); content/erasure closure (rows 12,13,14,15); clock anomalies
//! (row 24); anchors (rows 18,19,23; tier). Every check runs regardless of
//! whether an earlier Phase B check already found something — this is what
//! lets a single bundle with BOTH a tampered record and a malformed anchor
//! report both findings (Task 7 brief's required combo test).
//!
//! ## One success exit
//!
//! The crate's success variant is constructed in exactly ONE place in this
//! file: the final `Disposition::Clean => ...` arm at the end of
//! `verify_bundle_with_witness` (see that function's closing lines).
//! `tests/pipeline.rs::single_success_exit` asserts this by source
//! inspection (`include_str!` on this file, counting the exact token this
//! comment is deliberately avoiding spelling out, so the audited count
//! isn't inflated by its own documentation). Every other outcome in this
//! file constructs `Verdict::Tampered` or `Verdict::CannotVerify` — which
//! may appear as many times as needed (the constraint is specific to the
//! success variant, the direction a false result
//! would be catastrophic in).

use crate::chain::{chain_heads, parse_record, walk_chain, ChainError, RecordHeader};
use crate::checkpoint::{range_bounds, verify_checkpoint_chain, CheckpointChain, CpError};
use crate::container::{ContainerError, ContainerReader};
use crate::envelope::{verify_envelope, EnvelopeError, PT_RECORD};
use crate::hash::content_hash;
use crate::keys::{
    classify_compromise, key_at_position, verify_key_chain, CompromiseClass, KeyChain,
    KeyChainError,
};
use crate::manifest::{verify_manifest, verify_members, Manifest, ManifestError, MemberError};
use crate::strict::from_slice_strict;
use crate::verdict::{
    classify, AnchorTier, CountsSummary, Disposition, Finding, ScopeSummary, TimeSpan, Verdict,
    VerdictReport, NEGATIVE_SPACE,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashSet;

// ---- finding codes (row -> stable slug; see verdict.rs module docs) ----
const CODE_TAMPERED_SIGNATURE: &str = "tampered-signature";
const CODE_KEY_EPOCH: &str = "tampered-key-epoch";
const CODE_TAMPERED_CHAIN: &str = "tampered-chain";
const CODE_INVALID_ROTATION: &str = "tampered-invalid-rotation";
const CODE_UNAUTHORIZED_CONTROL: &str = "tampered-unauthorized-control-record";
const CODE_POST_COMPROMISE: &str = "tampered-post-compromise-position";
const CODE_VALID_PRE_CLAIM: &str = "valid-pre-claim";
const CODE_INDETERMINATE: &str = "cannot-verify-compromise-window-indeterminate";
const CODE_SCOPE: &str = "tampered-scope";
const CODE_TAMPERED_MANIFEST: &str = "tampered-manifest";
const CODE_TAMPERED_CONTENT: &str = "tampered-content";
const CODE_WITHHELD_ERASED: &str = "withheld-erased";
const CODE_BUNDLE_INCOMPLETE: &str = "cannot-verify-bundle-incomplete";
const CODE_ERASURE_INCONSISTENT: &str = "cannot-verify-erasure-inconsistent";
const CODE_UNKNOWN_KEY: &str = "cannot-verify-unknown-key";
const CODE_UNSUPPORTED: &str = "cannot-verify-unsupported-format";
const CODE_ANCHOR_INVALID: &str = "anchor-invalid";
const CODE_ANCHOR_UNLINKED: &str = "anchor-unlinked";
const CODE_CONTAINER_PROFILE: &str = "cannot-verify-container-profile";
const CODE_MALFORMED: &str = "cannot-verify-malformed";
const CODE_TIME_CLAIM_FALSIFIED: &str = "time-claim-falsified";
const CODE_CLOCK_ANOMALY: &str = "clock-anomaly";
const CODE_TRIVIAL_RANGE: &str = "trivial-range";

/// THE pipeline (pinned public signature — Task 7 brief). Always verifies
/// with an empty trusted-witness-time set; see
/// [`verify_bundle_with_witness`] for the (v1, R4-scoped) witness-time
/// override that row 8 needs.
pub fn verify_bundle(bytes: &[u8]) -> VerdictReport {
    verify_bundle_with_witness(bytes, &[])
}

/// Same pipeline, plus an explicit, OPERATOR-SUPPLIED set of
/// `(checkpoint_position, trusted_time)` assertions — "I personally know/
/// trust that this checkpoint existed at or before this time," entirely
/// out-of-band from the bundle's own bytes. This exists because v1 (R4)
/// never cryptographically validates a real TSA/QTSP anchor token, so
/// `anchored_before_claim` (spec: "row-8 time comparisons only ever use
/// validated anchors") can otherwise never be `true` in v1 — row 8
/// (`ValidPreClaim`) would be permanently unreachable through
/// `verify_bundle` alone. This function is never fed anything derived from
/// the bundle itself; passing an empty slice (what `verify_bundle` does)
/// makes the two functions behave identically.
pub fn verify_bundle_with_witness(
    bytes: &[u8],
    trusted_witness_times: &[(u64, DateTime<Utc>)],
) -> VerdictReport {
    // ---- Phase A: sequential, fail-fast bootstrap ----
    let reader = match ContainerReader::open(bytes) {
        Ok(r) => r,
        Err(e) => return terminal(container_error_finding(e), None),
    };

    let key_lines = match required_member_lines(&reader, "trust/keys.jsonl") {
        Ok(l) => l,
        Err(f) => return terminal(f, None),
    };
    let key_refs: Vec<&[u8]> = key_lines.iter().map(|l| l.as_slice()).collect();
    let keys = match verify_key_chain(&key_refs) {
        Ok(k) => k,
        Err(e) => return terminal(key_chain_error_finding(e), None),
    };

    let manifest = match verify_manifest(&reader, &keys) {
        Ok(m) => m,
        Err(e) => return terminal(manifest_error_finding(e), None),
    };

    if let Err(e) = verify_members(&reader, &manifest) {
        return terminal(member_error_finding(e), Some(&manifest));
    }

    let cp_lines = match required_member_lines(&reader, "checkpoints.jsonl") {
        Ok(l) => l,
        Err(f) => return terminal(f, Some(&manifest)),
    };
    let cp_refs: Vec<&[u8]> = cp_lines.iter().map(|l| l.as_slice()).collect();
    let checkpoints = match verify_checkpoint_chain(&cp_refs, &keys) {
        Ok(c) => c,
        Err(e) => return terminal(cp_error_finding(e), Some(&manifest)),
    };

    let bounds = match range_bounds(&checkpoints, manifest.scope().range()) {
        Ok(b) => b,
        Err(e) => return terminal(cp_error_finding(e), Some(&manifest)),
    };

    // ---- Phase B: findings accumulate ----
    let mut findings: Vec<Finding> = Vec::new();

    let ecp = manifest.export_checkpoint();
    if ecp.position() != bounds.end_position
        || ecp.head() != bounds.end_head
        || ecp.count() != bounds.end_count
    {
        findings.push(Finding::new(
            10,
            CODE_SCOPE,
            "manifest export_checkpoint disagrees with the independently verified checkpoint chain",
        ));
    }

    let epoch_lines = collect_epoch_lines(&reader);
    let epoch_refs: Vec<&[u8]> = epoch_lines.iter().map(|l| l.as_slice()).collect();

    // B1: per-record signature/keyid/payloadType (rows 2,3,16,17,22).
    let mut record_infos: Vec<RecordAnalysis> = Vec::new();
    for line in &epoch_refs {
        match analyze_record(line, &keys) {
            Ok(info) => {
                if !info.position_correct {
                    findings.push(Finding::at(
                        3,
                        CODE_KEY_EPOCH,
                        "record signed by a real chain key, but not the one authoritative at its declared position",
                        info.position,
                    ));
                }
                record_infos.push(info);
            }
            Err(finding) => findings.push(finding),
        }
    }

    // B2: chain-link recomputation (row 4).
    match walk_chain(bounds.start_position, bounds.start_head, &epoch_refs) {
        Ok((final_pos, final_head)) => {
            if final_pos != bounds.end_position || final_head != bounds.end_head {
                findings.push(Finding::new(
                    4,
                    CODE_TAMPERED_CHAIN,
                    "final chain head/position does not land on the closing checkpoint",
                ));
            }
        }
        Err(e) => findings.push(chain_error_finding(e)),
    }

    // B3: manifest.counts.records vs the actual journal entry count (row 10).
    if manifest.counts().records() != epoch_lines.len() as u64 {
        findings.push(Finding::new(
            10,
            CODE_SCOPE,
            "manifest counts.records disagrees with the actual number of journal entries",
        ));
    }

    // B4: compromise classification (rows 7,8,9).
    if let Some(compromise) = keys.compromise() {
        for info in &record_infos {
            if !info.position_correct {
                continue; // row 3 already fired; compromise status is moot
            }
            if info.matched_vk_bytes != compromise.compromised_key {
                continue;
            }
            let anchored = anchored_before_claim_for(
                info.position,
                &checkpoints,
                compromise.claimed_compromise_time,
                trusted_witness_times,
            );
            match classify_compromise(&keys, info.position, &info.keyid_hex, anchored) {
                CompromiseClass::NotAffected => {}
                CompromiseClass::ValidPreClaim => findings.push(Finding::at(
                    8,
                    CODE_VALID_PRE_CLAIM,
                    "record predates the claimed compromise time under an operator-trusted witness",
                    info.position,
                )),
                CompromiseClass::TamperedPostPosition => findings.push(Finding::at(
                    7,
                    CODE_POST_COMPROMISE,
                    "record signed by the compromised key at or after its compromise position",
                    info.position,
                )),
                CompromiseClass::IndeterminateWindow => findings.push(Finding::at(
                    9,
                    CODE_INDETERMINATE,
                    "record's compromise window is indeterminate (unanchored, or anchored within [C, declaration])",
                    info.position,
                )),
            }
        }
    }

    // B5: content / erasure closure (rows 12,13,14,15; Task 7 Rulings F1/F2).
    //
    // F2 (dangling erasure), IN-RANGE erasure records — controller fix round
    // 1 (2026-08-25): the target requirement is NARROWER than CLOSURE
    // records get below. An honest in-range erasure record can legitimately
    // target a record that predates this export's window entirely (the
    // mainstream GDPR pattern: erase old content now, later export only a
    // recent compliance window — the erasure event is in-range, its long-
    // erased target is not, and that is not itself suspicious). Since an
    // erasure can only ever target content that already existed, `target`
    // is structurally always < the erasure record's own position ≤
    // `bounds.end_position` for any honestly-formed record — so the only
    // ways `target` can be genuinely impossible are `target == 0` (no real
    // record is ever journaled at the virtual genesis anchor) or `target >
    // bounds.end_position` (a forward reference to content that cannot yet
    // exist at export time). A `target` at or before `bounds.start_position`
    // (and nonzero) is simply outside this bundle's own content — nothing
    // to withhold HERE, but not dangling either — no finding.
    let in_range_positions: HashSet<u64> = record_infos.iter().map(|info| info.position).collect();

    let mut erasure_targets: HashSet<u64> = HashSet::new();
    for info in &record_infos {
        if info.kind == "erasure" {
            match serde_json::from_value::<ErasureBody>(info.body.clone()) {
                Ok(b) if in_range_positions.contains(&b.target) => {
                    erasure_targets.insert(b.target);
                }
                Ok(b) if b.target != 0 && b.target <= bounds.start_position => {
                    // Benign: an honest erasure of a record outside this
                    // export's own window (see comment above) — not this
                    // bundle's content to resolve, not dangling.
                }
                Ok(b) => findings.push(Finding::at(
                    22,
                    CODE_MALFORMED,
                    format!("erasure record targets position {} which is not a valid predecessor position (0, or past the export's own tip)", b.target),
                    info.position,
                )),
                Err(_) => findings.push(Finding::at(
                    22,
                    CODE_MALFORMED,
                    "erasure record body missing/invalid target",
                    info.position,
                )),
            }
        }
    }

    // F1 (closure.jsonl interpretation): erasure records targeting in-range
    // content whose OWN position is outside the exported range (design doc
    // "erasure closure" — a later re-export of an older window still needs
    // to show a since-erased record as withheld, not incomplete).
    // Authenticated via the SAME per-record checks as any epoch record
    // (`analyze_record`: envelope signature, payloadType, key-position-
    // validity) — chain MEMBERSHIP is deliberately NOT re-verified, since a
    // closure record's neighbors are outside the exported range by
    // definition. Accepting a signed-but-not-chain-verified erasure record
    // can only ever WEAKEN a claim (content -> withheld); it can never
    // fabricate a false clean verdict of content, because content presence/
    // absence is independently re-derived from the container bytes below.
    let closure_lines = collect_member_lines(&reader, "journal/closure.jsonl");
    let closure_refs: Vec<&[u8]> = closure_lines.iter().map(|l| l.as_slice()).collect();
    for line in &closure_refs {
        match analyze_record(line, &keys) {
            Ok(info) => {
                if !info.position_correct {
                    findings.push(Finding::at(
                        3,
                        CODE_KEY_EPOCH,
                        "closure record signed by a real chain key, but not the one authoritative at its declared position",
                        info.position,
                    ));
                }
                if info.kind != "erasure" {
                    findings.push(Finding::at(
                        22,
                        CODE_MALFORMED,
                        format!("journal/closure.jsonl may only carry erasure records, found kind {:?}", info.kind),
                        info.position,
                    ));
                    continue;
                }
                if info.position > bounds.start_position && info.position <= bounds.end_position {
                    findings.push(Finding::at(
                        22,
                        CODE_MALFORMED,
                        "closure record's own position must be strictly outside the exported range",
                        info.position,
                    ));
                    continue;
                }
                match serde_json::from_value::<ErasureBody>(info.body.clone()) {
                    Ok(b) if in_range_positions.contains(&b.target) => {
                        erasure_targets.insert(b.target);
                    }
                    Ok(b) => findings.push(Finding::at(
                        22,
                        CODE_MALFORMED,
                        format!("closure erasure record targets position {} which does not resolve to any record in the exported range", b.target),
                        info.position,
                    )),
                    Err(_) => findings.push(Finding::at(
                        22,
                        CODE_MALFORMED,
                        "closure erasure record body missing/invalid target",
                        info.position,
                    )),
                }
            }
            Err(finding) => findings.push(finding),
        }
    }
    if manifest.counts().closure() != closure_lines.len() as u64 {
        findings.push(Finding::new(
            10,
            CODE_SCOPE,
            "manifest closure count disagrees with the actual number of journal/closure.jsonl entries",
        ));
    }

    let mut actual_withheld = 0u64;
    for info in &record_infos {
        let Some(declared_hash) = info.content_hash else {
            continue;
        };
        let member_name = format!("content/{}", info.position);
        match reader.member_bytes(&member_name) {
            Ok(blob) if blob.len() >= 32 => {
                let (salt_bytes, content_bytes) = blob.split_at(32);
                let salt: [u8; 32] = salt_bytes
                    .try_into()
                    .expect("split_at(32) on a >=32-byte slice always yields a 32-byte head");
                let got = content_hash(&salt, content_bytes);
                if got != declared_hash {
                    findings.push(Finding::at(
                        12,
                        CODE_TAMPERED_CONTENT,
                        "content blob salted-hash mismatch",
                        info.position,
                    ));
                } else if erasure_targets.contains(&info.position) {
                    findings.push(Finding::at(
                        15,
                        CODE_ERASURE_INCONSISTENT,
                        "content present despite a journaled erasure record",
                        info.position,
                    ));
                }
            }
            Ok(_short) => findings.push(Finding::at(
                22,
                CODE_MALFORMED,
                "content blob shorter than the 32-byte salt",
                info.position,
            )),
            Err(_) => {
                if erasure_targets.contains(&info.position) {
                    findings.push(Finding::at(
                        13,
                        CODE_WITHHELD_ERASED,
                        "content withheld per a journaled erasure record",
                        info.position,
                    ));
                    actual_withheld += 1;
                } else {
                    findings.push(Finding::at(
                        14,
                        CODE_BUNDLE_INCOMPLETE,
                        "content missing and no matching erasure record",
                        info.position,
                    ));
                }
            }
        }
    }
    if manifest.counts().withheld_erased() != actual_withheld {
        findings.push(Finding::new(
            10,
            CODE_SCOPE,
            "manifest withheld_erased count disagrees with the actual number of withheld records",
        ));
    }

    // B6: wall-clock anomalies (row 24, informational).
    for anomaly in checkpoints.clock_anomalies() {
        findings.push(Finding::at(
            24,
            CODE_CLOCK_ANOMALY,
            format!(
                "checkpoint at position {} does not advance the wall clock past position {}",
                anomaly.position, anomaly.previous_position
            ),
            anomaly.position,
        ));
    }

    // B7: anchors (rows 18,19,23; tier — v1/R4: never validated to tier >= 2).
    let (anchor_tier, anchor_findings) = process_anchors(&reader, &checkpoints);
    findings.extend(anchor_findings);

    // B8: trivial range (row 25, informational).
    if manifest.counts().records() == 0 {
        findings.push(Finding::new(25, CODE_TRIVIAL_RANGE, "zero-record range export"));
    }

    let (disposition, dominant, findings) = classify(findings);
    // ONE success exit (tests/pipeline.rs::single_success_exit asserts this
    // by source inspection): the only place the success variant is
    // constructed in this file — everywhere else in this function returns
    // early via `terminal(...)`, which never constructs it.
    let verdict = match disposition {
        Disposition::Clean => Verdict::Valid,
        Disposition::Blocking(v) => v,
    };

    let time_confidence: Vec<TimeSpan> = checkpoints
        .checkpoints()
        .iter()
        .map(|cp| TimeSpan {
            label: format!("checkpoint at position {}", cp.position),
            // v1/R4: no anchor is ever cryptographically validated, so
            // every wall-clock claim in this report is self-asserted —
            // never overclaimed as anchor-bounded.
            anchored: false,
            at: cp.at.to_rfc3339(),
        })
        .collect();

    VerdictReport {
        verdict,
        dominant,
        findings,
        anchor_tier,
        scope: manifest_scope_summary(&manifest),
        counts: manifest_counts_summary(&manifest),
        negative_space: NEGATIVE_SPACE,
        time_confidence,
    }
}

// ---- Additive read helpers for offline tooling (CLI why/tables/--against) ----
//
// These do NO trust evaluation of their own — every caller MUST run
// `verify_bundle` first and gate on a `Valid` verdict before interpreting
// anything they return. They exist so the CLI never reparses `.dbev` bytes
// or recomputes a domain-separated hash itself (the parser-differential risk
// the design's Round-5 N4 warns against): they reuse the SAME container
// reader, per-line record parser, and chain walker the verifier uses. Trust
// logic stays here, in the audited MIT crate — not in the CLI.

/// Every way a read helper can fail to even parse a bundle far enough to read
/// its records/heads. Distinct from the verdict taxonomy (`verdict.rs`),
/// which classifies an already-parsed bundle's authenticity — a value of this
/// type means "these bytes are not a well-formed `.dbev` at all," which for a
/// bundle that already passed `verify_bundle` is structurally unreachable.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("container: {0}")]
    Container(#[from] ContainerError),
    #[error("key chain: {0}")]
    KeyChain(#[from] KeyChainError),
    #[error("manifest: {0}")]
    Manifest(#[from] ManifestError),
    #[error("checkpoint chain: {0}")]
    Checkpoint(#[from] CpError),
    #[error("record chain: {0}")]
    Chain(#[from] ChainError),
    #[error("required container member {0:?} is absent")]
    MissingMember(String),
    #[error("verified key chain has no genesis signing key")]
    NoGenesisKey,
}

/// A bundle's cross-bundle comparison surface: its stable journal identity
/// plus the per-position running heads. Both come from ONE bootstrap so a
/// caller (the CLI's `--against`) never opens or re-verifies the container
/// twice, and identity derivation stays inside the audited crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleHeads {
    /// The journal's identity: its genesis signing key's public-key bytes —
    /// the self-signed TOFU root at key-chain position 0, immutable for the
    /// life of the journal (`KeyChain::all_signing_keys().next()`). Two
    /// exports of the SAME journal share this; two unrelated journals — and a
    /// post-compromise successor-genesis lineage — do not. `--against` gates a
    /// FORK verdict on this matching FIRST, so a head disagreement between two
    /// UNRELATED journals is never miscalled a fork (a false fraud
    /// accusation), only ever "different journals / not-comparable".
    pub genesis_identity: [u8; 32],
    /// Per-position `(position, head_after_position)`, prefixed with the
    /// range's start anchor (see [`chain_heads_for_bundle`]).
    pub heads: Vec<(u64, [u8; 32])>,
}

/// Reads every `journal/epoch-*.jsonl` record of a `.dbev` container, in the
/// SAME ascending epoch-number/line order the verifier walks, parsed into
/// closed-schema [`RecordHeader`]s. For the CLI's `evidence why`/`tables`,
/// which render decision/premise record fields — call `verify_bundle` first
/// and refuse on any non-`Valid` verdict; the records this returns are only
/// trustworthy once that verdict is `Valid`.
pub fn read_records(bytes: &[u8]) -> Result<Vec<RecordHeader>, ReadError> {
    let reader = ContainerReader::open(bytes)?;
    let lines = collect_epoch_lines(&reader);
    let mut out = Vec::with_capacity(lines.len());
    for line in &lines {
        out.push(parse_record(line)?);
    }
    Ok(out)
}

/// The per-position `(position, head_after_position)` commitments for a
/// bundle's exported journal range, prefixed with the range's start anchor
/// `(start_position, start_head)`. Computed by the SAME bootstrap the
/// verifier uses (container → key chain → manifest → checkpoint chain →
/// range bounds → [`chain_heads`]), so no second walk can drift from the
/// trust core.
///
/// This is the input to the CLI's `--against` cross-bundle consistency check.
/// The start anchor is included so a windowed export's signed start
/// checkpoint participates directly in the overlap comparison; the comparison
/// itself excludes the universal genesis anchor (position 0 / all-zero head),
/// which carries no journal identity. The returned [`BundleHeads`] ALSO
/// carries the journal's genesis identity from this same bootstrap, so the
/// caller can gate a FORK verdict on same-journal identity before ever
/// comparing heads. As with [`read_records`], callers MUST gate on a `Valid`
/// verdict from `verify_bundle` before asserting any consistency claim: a
/// `Tampered` bundle can still produce heads here.
pub fn chain_heads_for_bundle(bytes: &[u8]) -> Result<BundleHeads, ReadError> {
    let reader = ContainerReader::open(bytes)?;

    let key_lines = read_member_lines(&reader, "trust/keys.jsonl")?;
    let key_refs: Vec<&[u8]> = key_lines.iter().map(|l| l.as_slice()).collect();
    let keys = verify_key_chain(&key_refs)?;
    // The journal's stable identity, surfaced from the SAME verified key
    // chain the head walk uses (never re-derived, never computed in CLI
    // code). `events[0]` is always genesis for a verified chain; the `None`
    // arm is structurally unreachable, handled rather than unwrapped.
    let genesis_identity = keys
        .all_signing_keys()
        .next()
        .map(|vk| vk.to_bytes())
        .ok_or(ReadError::NoGenesisKey)?;

    let manifest = verify_manifest(&reader, &keys)?;

    let cp_lines = read_member_lines(&reader, "checkpoints.jsonl")?;
    let cp_refs: Vec<&[u8]> = cp_lines.iter().map(|l| l.as_slice()).collect();
    let checkpoints = verify_checkpoint_chain(&cp_refs, &keys)?;

    let bounds = range_bounds(&checkpoints, manifest.scope().range())?;

    let epoch_lines = collect_epoch_lines(&reader);
    let epoch_refs: Vec<&[u8]> = epoch_lines.iter().map(|l| l.as_slice()).collect();
    let walked = chain_heads(bounds.start_position, bounds.start_head, &epoch_refs)?;

    let mut heads = Vec::with_capacity(walked.len() + 1);
    heads.push((bounds.start_position, bounds.start_head));
    heads.extend(walked);
    Ok(BundleHeads { genesis_identity, heads })
}

/// [`required_member_lines`]' sibling for the [`ReadError`] world: read a
/// required member's lines, or fail with [`ReadError::MissingMember`].
fn read_member_lines(reader: &ContainerReader, name: &str) -> Result<Vec<Vec<u8>>, ReadError> {
    match reader.member_bytes(name) {
        Ok(bytes) => Ok(split_lines(bytes)),
        Err(_) => Err(ReadError::MissingMember(name.to_string())),
    }
}

// ---- Phase A helpers ----

fn manifest_scope_summary(m: &Manifest) -> ScopeSummary {
    ScopeSummary {
        range: m.scope().range(),
        classes: m.scope().classes().to_vec(),
        spaces: m.scope().spaces().map(|s| s.to_vec()),
    }
}

fn manifest_counts_summary(m: &Manifest) -> CountsSummary {
    CountsSummary {
        records: m.counts().records(),
        closure: m.counts().closure(),
        withheld_erased: m.counts().withheld_erased(),
    }
}

/// Builds a terminal (single-finding) report for a Phase A failure.
/// `manifest`, when available, supplies honest scope/counts even though
/// the overall verdict is non-VALID (spec law 5 disclosures are not
/// conditioned on the verdict).
fn terminal(finding: Finding, manifest: Option<&Manifest>) -> VerdictReport {
    let (disposition, dominant, findings) = classify(vec![finding]);
    let verdict = match disposition {
        Disposition::Blocking(v) => v,
        Disposition::Clean => {
            // Invariant this depends on: every `terminal()` call site feeds a
            // BLOCKING-row finding (a Phase A `*_error_finding` mapping), so
            // `classify()` never returns `Clean` here. If a future
            // `*_error_finding` helper ever emitted an INFORMATIONAL row
            // through `terminal()`, this would panic — fails loud, never a
            // false VALID. Cross-language asymmetry to preserve: the Python
            // `_terminal` (verify_dbev.py) falls back to VALID on the same
            // violation, so keep all `terminal()` findings blocking rows on
            // both sides — here it crashes loudly, there it would go silently
            // VALID.
            unreachable!("a single non-empty finding can never classify as Clean")
        }
    };
    let (scope, counts) = match manifest {
        Some(m) => (manifest_scope_summary(m), manifest_counts_summary(m)),
        None => (
            ScopeSummary {
                range: (0, 0),
                classes: Vec::new(),
                spaces: None,
            },
            CountsSummary {
                records: 0,
                closure: 0,
                withheld_erased: 0,
            },
        ),
    };
    VerdictReport {
        verdict,
        dominant,
        findings,
        anchor_tier: AnchorTier::None,
        scope,
        counts,
        negative_space: NEGATIVE_SPACE,
        time_confidence: Vec::new(),
    }
}

fn split_lines(bytes: &[u8]) -> Vec<Vec<u8>> {
    bytes
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| l.to_vec())
        .collect()
}

fn required_member_lines(reader: &ContainerReader, name: &str) -> Result<Vec<Vec<u8>>, Finding> {
    match reader.member_bytes(name) {
        Ok(bytes) => Ok(split_lines(bytes)),
        Err(_) => Err(Finding::new(
            21,
            CODE_CONTAINER_PROFILE,
            format!("required member {name:?} is absent from the container"),
        )),
    }
}

/// Every `journal/epoch-<N>.jsonl` member, concatenated in ascending
/// epoch-number order. Absent entirely is valid (a zero-record range).
fn collect_epoch_lines(reader: &ContainerReader) -> Vec<Vec<u8>> {
    let mut epoch_files: Vec<(u64, &str)> = reader
        .member_names()
        .iter()
        .filter_map(|n| {
            let rest = n.strip_prefix("journal/epoch-")?.strip_suffix(".jsonl")?;
            rest.parse::<u64>().ok().map(|num| (num, n.as_str()))
        })
        .collect();
    epoch_files.sort_by_key(|(num, _)| *num);
    let mut lines = Vec::new();
    for (_, name) in epoch_files {
        // Structurally unreachable: `name` came from `member_names()`,
        // which only ever lists names `ContainerReader::open` already
        // indexed under the SAME key (manifest.rs's `verify_members` notes
        // the identical invariant) — handled instead of `.expect()`-ed to
        // keep the no-unwrap-in-prod bar absolute even on this
        // never-taken branch.
        let Ok(bytes) = reader.member_bytes(name) else {
            continue;
        };
        lines.extend(split_lines(bytes));
    }
    lines
}

/// `journal/closure.jsonl`'s lines, tolerant of the member being absent
/// entirely — same tolerance [`collect_epoch_lines`] applies to
/// `journal/epoch-*.jsonl` (a bundle with nothing to carry forward, or one
/// built before this member existed, is not itself a container-profile
/// violation).
fn collect_member_lines(reader: &ContainerReader, name: &str) -> Vec<Vec<u8>> {
    match reader.member_bytes(name) {
        Ok(bytes) => split_lines(bytes),
        Err(_) => Vec::new(),
    }
}

fn container_error_finding(e: ContainerError) -> Finding {
    // Task 6's own doc pin (container.rs module docs): every variant maps
    // to CANNOT_VERIFY(container-profile), row 21 — never TAMPERED.
    Finding::new(21, CODE_CONTAINER_PROFILE, e.to_string())
}

fn key_chain_error_finding(e: KeyChainError) -> Finding {
    let detail = e.to_string();
    match e {
        KeyChainError::Malformed { .. }
        | KeyChainError::MissingGenesis
        | KeyChainError::GenesisPositionMismatch { .. }
        | KeyChainError::DuplicateGenesis { .. }
        | KeyChainError::UnknownKind { .. }
        | KeyChainError::InvalidCompromiseTime { .. } => Finding::new(22, CODE_MALFORMED, detail),
        KeyChainError::GenesisNotSelfSigned => Finding::new(2, CODE_TAMPERED_SIGNATURE, detail),
        KeyChainError::PositionNotIncreasing { .. } | KeyChainError::KeyLinkMismatch { .. } => {
            Finding::new(4, CODE_TAMPERED_CHAIN, detail)
        }
        KeyChainError::UnauthorizedRotation { position } => {
            Finding::at(5, CODE_INVALID_ROTATION, detail, position)
        }
        KeyChainError::UnauthorizedControlRecord { position } => {
            Finding::at(6, CODE_UNAUTHORIZED_CONTROL, detail, position)
        }
        KeyChainError::JournalSealed { sealed_at, .. } => {
            Finding::at(7, CODE_POST_COMPROMISE, detail, sealed_at)
        }
    }
}

fn manifest_error_finding(e: ManifestError) -> Finding {
    let detail = e.to_string();
    match e {
        ManifestError::Missing => Finding::new(21, CODE_CONTAINER_PROFILE, detail),
        ManifestError::Malformed(_) => Finding::new(22, CODE_MALFORMED, detail),
        ManifestError::Signature => Finding::new(11, CODE_TAMPERED_MANIFEST, detail),
    }
}

fn member_error_finding(e: MemberError) -> Finding {
    let detail = e.to_string();
    match e {
        MemberError::HashMismatch { .. } => Finding::new(12, CODE_TAMPERED_CONTENT, detail),
        MemberError::UnlistedMember { .. } | MemberError::MissingMember { .. } => {
            Finding::new(21, CODE_CONTAINER_PROFILE, detail)
        }
    }
}

fn cp_error_finding(e: CpError) -> Finding {
    let detail = e.to_string();
    match e {
        CpError::Empty | CpError::Malformed { .. } | CpError::InvalidTimestamp { .. } => {
            Finding::new(22, CODE_MALFORMED, detail)
        }
        CpError::CpLinkMismatch { position }
        | CpError::PositionNotIncreasing { found: position, .. }
        | CpError::UnauthorizedSigner { position }
        | CpError::SignatureInvalid { position } => {
            Finding::at(4, CODE_TAMPERED_CHAIN, detail, position)
        }
        CpError::NotABoundary { position } => Finding::at(10, CODE_SCOPE, detail, position),
    }
}

fn chain_error_finding(e: ChainError) -> Finding {
    let detail = e.to_string();
    match e {
        ChainError::Malformed { .. } => Finding::new(22, CODE_MALFORMED, detail),
        ChainError::LinkMismatch { position } | ChainError::PositionDuplicate { position } => {
            Finding::at(4, CODE_TAMPERED_CHAIN, detail, position)
        }
        ChainError::PositionGap { found, .. } => Finding::at(4, CODE_TAMPERED_CHAIN, detail, found),
        ChainError::PositionOverflow { .. } => Finding::new(4, CODE_TAMPERED_CHAIN, detail),
    }
}

// ---- Phase B: per-record analysis ----

struct RecordAnalysis {
    position: u64,
    keyid_hex: String,
    matched_vk_bytes: [u8; 32],
    kind: String,
    content_hash: Option<[u8; 32]>,
    body: serde_json::Value,
    /// `false` when the record's signature verified under a REAL chain
    /// key, but not the one `key_at_position` resolves for its own
    /// declared position (row 3).
    position_correct: bool,
}

/// Tolerant top-level envelope peek — mirrors the `PayloadPeek` pattern
/// already used in `chain.rs`/`keys.rs`/`checkpoint.rs`, extended to also
/// read `payloadType`, `keyid`, and `signatures` (needed here, not there,
/// because this is the one place that must distinguish rows 2/3/16/17/22
/// from each other rather than delegating entirely to `verify_envelope`).
#[derive(Deserialize)]
struct EnvPeek {
    #[serde(rename = "payloadType")]
    payload_type: String,
    payload: String,
    #[serde(default)]
    keyid: Option<String>,
    #[serde(default)]
    signatures: Option<serde_json::Value>,
}

/// Closed-schema record payload, mirroring `chain::RecordHeader` field-for-
/// field but WITHOUT `deny_unknown_fields` (this is a peek to extract
/// `position`/`kind`/`content_hash`/`body`, not the trust boundary —
/// `walk_chain`, run separately over the same lines in Phase B, is what
/// enforces the closed schema for chain-linkage purposes).
#[derive(Deserialize)]
struct RecordPeek {
    position: u64,
    kind: String,
    #[serde(default)]
    content_hash: Option<String>,
    #[serde(default)]
    body: serde_json::Value,
}

#[derive(Deserialize)]
struct ErasureBody {
    target: u64,
}

/// Analyzes one record envelope line: payloadType/multi-sig/keyid checks
/// (rows 17,22), keyid recognition against the FULL key chain (row 16),
/// signature verification under the matched key (row 2), and position-vs-
/// key-epoch correctness (row 3, reported by the caller since it does not
/// block continuing to analyze this record). Returns `Err(Finding)` for
/// every other failure — one finding per record, at the first problem
/// found, since a record that fails basic authenticity cannot be trusted
/// for anything downstream (content/erasure/compromise checks skip it).
fn analyze_record(line: &[u8], keys: &KeyChain) -> Result<RecordAnalysis, Finding> {
    let peek: EnvPeek = from_slice_strict(line)
        .map_err(|e| Finding::new(22, CODE_MALFORMED, format!("record envelope JSON: {e}")))?;

    if peek.signatures.is_some() {
        return Err(Finding::new(
            17,
            CODE_UNSUPPORTED,
            "multi-signature record envelope (signatures array)",
        ));
    }
    if peek.payload_type != PT_RECORD {
        return Err(Finding::new(
            22,
            CODE_MALFORMED,
            format!("record payloadType mismatch: got {:?}", peek.payload_type),
        ));
    }
    let payload_bytes = STANDARD
        .decode(peek.payload.as_bytes())
        .map_err(|e| Finding::new(22, CODE_MALFORMED, format!("record payload base64: {e}")))?;
    let header: RecordPeek = from_slice_strict(&payload_bytes)
        .map_err(|e| Finding::new(22, CODE_MALFORMED, format!("record payload JSON: {e}")))?;
    let position = header.position;

    let keyid_hex = peek
        .keyid
        .ok_or_else(|| Finding::at(22, CODE_MALFORMED, "record envelope missing keyid", position))?;
    let keyid_bytes: [u8; 32] = hex::decode(&keyid_hex)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| {
            Finding::at(22, CODE_MALFORMED, "record keyid is not valid 32-byte hex", position)
        })?;

    let matched_vk = keys
        .all_signing_keys()
        .find(|vk| vk.to_bytes() == keyid_bytes)
        .ok_or_else(|| {
            Finding::at(
                16,
                CODE_UNKNOWN_KEY,
                "record signed by a key not reachable through the in-band key chain",
                position,
            )
        })?;

    match verify_envelope(line, PT_RECORD, matched_vk) {
        Ok(_) => {}
        Err(EnvelopeError::SignatureInvalid) => {
            return Err(Finding::at(
                2,
                CODE_TAMPERED_SIGNATURE,
                "record signature invalid under its declared (recognized) key",
                position,
            ));
        }
        Err(other) => {
            return Err(Finding::at(22, CODE_MALFORMED, format!("record envelope: {other}"), position));
        }
    }

    let content_hash = match header.content_hash {
        None => None,
        Some(hex_str) => Some(
            hex::decode(&hex_str)
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| {
                    Finding::at(22, CODE_MALFORMED, "record content_hash is not valid 32-byte hex", position)
                })?,
        ),
    };

    let position_correct = key_at_position(keys, position)
        .map(|vk| vk.to_bytes())
        == Some(matched_vk.to_bytes());

    Ok(RecordAnalysis {
        position,
        keyid_hex,
        matched_vk_bytes: matched_vk.to_bytes(),
        kind: header.kind,
        content_hash,
        body: header.body,
        position_correct,
    })
}

fn anchored_before_claim_for(
    position: u64,
    checkpoints: &CheckpointChain,
    claim: DateTime<Utc>,
    trusted_witness_times: &[(u64, DateTime<Utc>)],
) -> bool {
    let Some(covering) = checkpoints.checkpoints().iter().find(|cp| cp.position >= position) else {
        return false;
    };
    trusted_witness_times
        .iter()
        .any(|(cp_pos, t)| *cp_pos == covering.position && *t < claim)
}

// ---- Phase B: anchors (rows 18,19,23; tier — R4: plumbing only, real
// TSA/QTSP crypto validation is Task 17's) ----

#[derive(Deserialize)]
struct AnchorPeek {
    kind: String,
    checkpoint_position: u64,
    #[serde(default)]
    tsa_time: Option<String>,
}

fn higher_tier(a: AnchorTier, b: AnchorTier) -> AnchorTier {
    fn rank(t: AnchorTier) -> u8 {
        match t {
            AnchorTier::None => 0,
            AnchorTier::WitnessFilePresent => 1,
            AnchorTier::TokenPresentUnvalidated => 2,
        }
    }
    if rank(b) > rank(a) {
        b
    } else {
        a
    }
}

fn process_anchors(reader: &ContainerReader, checkpoints: &CheckpointChain) -> (AnchorTier, Vec<Finding>) {
    let mut tier = AnchorTier::None;
    let mut findings = Vec::new();

    for name in reader.member_names() {
        if !name.starts_with("anchors/") {
            continue;
        }
        let Ok(bytes) = reader.member_bytes(name) else {
            continue; // structurally unreachable: member_names() always resolves
        };
        // Strict-JSON pre-validate the anchor member (the ONE bundle parse with
        // no `deny_unknown_fields` closed-schema struct to backstop it) BEFORE
        // the tolerant `AnchorPeek` parse — parity with the Python verifier's
        // `json_parse` on the same bytes. Without the pre-pass a duplicate-key /
        // out-of-range-number / non-finite / over-deep anchor reads VALID here
        // (AnchorPeek silently skips or keeps-last) while Python reads
        // `anchor-invalid`, diverging on the dominant code. Any non-conformance
        // is malformed → the SAME row-18 outcome (both verifiers agree on 18).
        let anchor: AnchorPeek = match from_slice_strict(bytes) {
            Ok(a) => a,
            Err(_) => {
                findings.push(Finding::new(18, CODE_ANCHOR_INVALID, format!("anchor {name} failed to parse")));
                continue;
            }
        };
        let Some(checkpoint) = checkpoints
            .checkpoints()
            .iter()
            .find(|cp| cp.position == anchor.checkpoint_position)
        else {
            findings.push(Finding::new(
                19,
                CODE_ANCHOR_UNLINKED,
                format!(
                    "anchor {name} references checkpoint position {} not in the bundle's chain",
                    anchor.checkpoint_position
                ),
            ));
            continue;
        };

        // Structurally present and linked to a real checkpoint — v1 (R4)
        // never validates the token/witness cryptographically, so this is
        // as far as tier can go.
        let this_tier = match anchor.kind.as_str() {
            "witness" => AnchorTier::WitnessFilePresent,
            _ => AnchorTier::TokenPresentUnvalidated,
        };
        tier = higher_tier(tier, this_tier);

        if let Some(tsa_time) = &anchor.tsa_time
            && let Ok(tsa) = DateTime::parse_from_rfc3339(tsa_time).map(|d| d.with_timezone(&Utc))
            && checkpoint.at > tsa
        {
            findings.push(Finding::at(
                23,
                CODE_TIME_CLAIM_FALSIFIED,
                format!(
                    "checkpoint {} wall-clock is later than anchor {name}'s TSA time (provably false time claim)",
                    checkpoint.position
                ),
                checkpoint.position,
            ));
        }
    }

    (tier, findings)
}
