// SPDX-License-Identifier: MIT
//! Mutation gate (Task 16): the exhaustive every-byte-flip harness plus a set
//! of structural mutations, standing behind the hand-picked golden corpus
//! (Task 15). The corpus proves agreement on 23 curated rows; this proves the
//! Rust verifier never emits a false-VALID under adversarial single-byte and
//! structural tampering of an honest bundle.
//!
//! ## The invariant (controller ruling, 2026-08-25)
//!
//! A `.dbev` is a STORE-only ZIP wrapper, and a handful of bytes are inert to
//! the verifier's INTERPRETATION even though they live in the file:
//!   * ZIP framing the reader (`container.rs`) never interprets — a member's
//!     CRC-32, version fields, mod-time/date, and the central-directory
//!     attribute fields. Flipping one keeps the member view byte-identical.
//!   * The manifest DSSE envelope's `keyid` value. The manifest verifier
//!     resolves the signing key by `export_checkpoint.position`
//!     (`key_at_position`), NOT by the envelope's keyid, and `verify_envelope`
//!     only checks keyid is PRESENT — its value is neither authenticated nor
//!     interpreted (identically in the Rust and Python verifiers). Flipping it
//!     changes a member's bytes but changes NOTHING the bundle asserts, and an
//!     attacker gains nothing (the signature must still verify under the
//!     position-resolved key).
//!
//! So the correct inertness proof is not raw member-byte-identity (which the
//! manifest-keyid class fails) but the precise form of the controller's stated
//! goal — "the verifier's view of the evidence is unchanged": the FULL
//! `VerdictReport` (verdict + every finding + scope + counts + anchor tier +
//! time confidence) is identical before and after the flip. The invariant is
//! therefore:
//!
//!   for every flipped offset, `verify_bundle` returns a deterministic verdict;
//!   and IF that verdict is VALID, the flip MUST be provably inert — the whole
//!   VerdictReport is byte-for-byte identical to the baseline. An inert VALID
//!   survivor is acceptable and asserted as such; a VALID survivor whose report
//!   CHANGED would mean the bundle now asserts something different yet still
//!   verifies VALID — an unauthenticated MEANINGFUL byte, a CRITICAL
//!   false-VALID gap — and this harness panics on it rather than papering over
//!   it.
//!
//! This is sound because every byte the verifier authenticates is inside a
//! signed envelope or a salted-hashed content blob: any change to an asserted
//! fact breaks a signature or a hash (→ non-VALID, caught) OR would surface as
//! a changed finding/scope/count in the report (→ report differs, caught). The
//! only VALID survivors are the inert framing / manifest-keyid bytes, each
//! proven inert by report-identity.

use docbrain_evidence::{verify_bundle, BundleBuilder, ContainerReader, ContainerWriter, Verdict};

/// A small (< 64KB) but representative honest bundle: multiple records, a
/// content blob, an honest erasure (so `journal/closure.jsonl`, content
/// members, and a withheld record all participate), and a linked anchor.
fn small_valid_bundle() -> Vec<u8> {
    BundleBuilder::new()
        .add_records(3)
        .with_content(1, b"first record content")
        .with_content(2, b"second record content")
        .erase(2)
        .with_anchor_witness(3)
        .build()
}

/// The member view the verifier actually consumes: every member name mapped to
/// its exact bytes, order-normalized. `None` if the container does not even
/// open. Two bundles with an equal view are indistinguishable to
/// `verify_bundle` (it only ever reads members by name), so an equal view is a
/// complete proof that a mutation is semantically inert.
fn member_view(bytes: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    let reader = ContainerReader::open(bytes).ok()?;
    let mut view: Vec<(String, Vec<u8>)> = reader
        .member_names()
        .iter()
        .map(|name| {
            let data = reader
                .member_bytes(name)
                .expect("a name from member_names always resolves")
                .to_vec();
            (name.clone(), data)
        })
        .collect();
    view.sort_by(|a, b| a.0.cmp(&b.0));
    Some(view)
}

#[test]
fn every_single_byte_flip_is_non_valid_or_provably_inert() {
    let original = small_valid_bundle();
    assert!(
        original.len() < 64 * 1024,
        "keep the mutation bundle small; got {} bytes",
        original.len()
    );
    let baseline = verify_bundle(&original);
    assert_eq!(baseline.verdict, Verdict::Valid, "the un-mutated baseline must be VALID");
    let baseline_view = member_view(&original).expect("baseline must open");

    // Two distinct single-byte mutations per offset: XOR 0xFF (flip every bit)
    // and wrapping +1. Both always change the byte, so neither can accidentally
    // re-test the unmutated bundle.
    let mutators: [(&str, fn(u8) -> u8); 2] =
        [("xor0xFF", |b| b ^ 0xFF), ("add1", |b| b.wrapping_add(1))];

    let mut valid_survivors = 0usize; // report-identical VALID survivors
    let mut view_identical_survivors = 0usize; // subset: also member-byte-identical
    let mut total = 0usize;

    for offset in 0..original.len() {
        for (label, mutate) in mutators {
            let mut mutated = original.clone();
            mutated[offset] = mutate(mutated[offset]);
            total += 1;
            let report = verify_bundle(&mutated);
            if report.verdict == Verdict::Valid {
                // A VALID survivor is acceptable ONLY if it is provably inert:
                // the whole VerdictReport must be identical, i.e. the flip
                // changed nothing the verifier interprets or asserts. Anything
                // else is a CRITICAL false-VALID gap — fail loudly with the
                // offset and the byte it controls.
                assert!(
                    report == baseline,
                    "CRITICAL false-VALID: byte offset {offset} ({label}) verified VALID \
                     but CHANGED the verdict report — an unauthenticated MEANINGFUL byte. \
                     Original byte {:#04x} -> {:#04x}. dominant now row {} [{}].",
                    original[offset],
                    mutated[offset],
                    report.dominant.row,
                    report.dominant.code,
                );
                valid_survivors += 1;
                if member_view(&mutated).as_ref() == Some(&baseline_view) {
                    view_identical_survivors += 1;
                }
            }
        }
    }

    // Every VALID survivor is inert (asserted in-loop). Surface the tally so
    // the report can show the inert-byte footprint: ZIP framing (member view
    // unchanged) vs manifest-keyid (report unchanged, member bytes differ).
    let non_valid = total - valid_survivors;
    eprintln!(
        "mutation: {total} single-byte mutations over a {}-byte bundle; \
         {non_valid} non-VALID; {valid_survivors} VALID inert survivors \
         ({view_identical_survivors} member-view-identical [ZIP framing], \
         {} report-identical-only [manifest keyid]).",
        original.len(),
        valid_survivors - view_identical_survivors,
    );

    // Sanity: the overwhelming majority of flips must be caught. If inert
    // survivors ever exceeded a small fraction, the reader would be ignoring
    // far more than framing + the one decorative keyid field.
    assert!(
        valid_survivors * 8 < total,
        "too many VALID survivors ({valid_survivors} of {total}) — the verifier may \
         be ignoring semantically meaningful bytes"
    );
}

#[test]
fn structural_mutation_duplicate_member_is_non_valid() {
    // Two `checkpoints.jsonl` entries in the raw container — the reader rejects
    // duplicate names (container-profile), never silently picks one.
    let bytes = BundleBuilder::new()
        .add_records(3)
        .duplicate_member("checkpoints.jsonl")
        .build();
    assert_ne!(verify_bundle(&bytes).verdict, Verdict::Valid);
}

#[test]
fn structural_mutation_dropped_member_is_non_valid() {
    // A member declared in the manifest but physically absent — the member-hash
    // pass must not silently accept the gap.
    let bytes = BundleBuilder::new()
        .add_records(3)
        .missing_container_member("checkpoints.jsonl")
        .build();
    assert_ne!(verify_bundle(&bytes).verdict, Verdict::Valid);
}

#[test]
fn structural_mutation_unlisted_member_is_non_valid() {
    // A physically-present member the manifest never lists — the container/
    // manifest cross-check must reject it (no smuggled members).
    let bytes = BundleBuilder::new()
        .add_records(3)
        .unlisted_container_member()
        .build();
    assert_ne!(verify_bundle(&bytes).verdict, Verdict::Valid);
}

#[test]
fn structural_mutation_reordering_members_is_inert_valid() {
    // Reordering the physical container members carries ZERO evidence: the
    // reader is name-indexed and re-sorts epoch files, so a reorder that leaves
    // every member's bytes intact is provably inert and legitimately stays
    // VALID (the same class as the inert-framing byte flips above). This test
    // pins that property so a future change that made member ORDER load-bearing
    // — a real parser-differential risk against the Python reader — would fail
    // here.
    let original = small_valid_bundle();
    let baseline_view = member_view(&original).expect("baseline opens");

    let reader = ContainerReader::open(&original).expect("baseline opens");
    let original_order: Vec<String> = reader.member_names().to_vec();
    let mut reversed = original_order.clone();
    reversed.reverse();
    let mut writer = ContainerWriter::new();
    for name in &reversed {
        writer
            .add_member(name, reader.member_bytes(name).expect("resolves").to_vec())
            .expect("re-adding an existing whitelisted member always succeeds");
    }
    let reordered = writer.finish().expect("writer never exceeds limits");

    let reordered_order = ContainerReader::open(&reordered)
        .expect("reordered opens")
        .member_names()
        .to_vec();
    assert_ne!(
        original_order, reordered_order,
        "the physical order must actually differ for this to test anything"
    );
    assert_eq!(
        verify_bundle(&reordered).verdict,
        Verdict::Valid,
        "a pure member reorder carries no evidence and must stay VALID"
    );
    assert_eq!(
        member_view(&reordered),
        Some(baseline_view),
        "reorder must preserve every member's bytes (semantic identity)"
    );
}

#[test]
fn truncation_at_many_offsets_is_never_valid() {
    let original = small_valid_bundle();
    // ~15 truncation points across the file, plus the two degenerate ends.
    let mut cuts: Vec<usize> = (0..15).map(|i| original.len() * i / 15).collect();
    cuts.push(0);
    cuts.push(original.len().saturating_sub(1));
    for cut in cuts {
        let truncated = &original[..cut];
        // verify_bundle must fail closed (some non-VALID verdict) and must
        // never panic — a truncated archive can never authenticate.
        assert_ne!(
            verify_bundle(truncated).verdict,
            Verdict::Valid,
            "truncation at {cut} bytes must not verify VALID"
        );
    }
}
