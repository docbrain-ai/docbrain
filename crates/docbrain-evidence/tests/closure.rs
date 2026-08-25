// SPDX-License-Identifier: MIT
//! Task 12 pipeline tests: `verify_bundle`'s interpretation of
//! `journal/closure.jsonl` (Task 7 Ruling F1) plus the dangling-erasure-
//! target check (Ruling F2), on both the in-range and closure paths.
//!
//! Task 7 authenticated closure.jsonl's container shape (it is a normal
//! whitelisted member the container profile already accepts) but
//! deliberately never READ it — `collect_epoch_lines` only globs
//! `journal/epoch-*.jsonl`. This file proves the NEW interpretation: an
//! out-of-range erasure record targeting in-range content is honored
//! (row 13, not row 14); a dangling target (in-range OR via closure) is
//! rejected (row 22), never silently accepted; content present despite a
//! journaled erasure is still row 15 regardless of which member carried
//! the erasure record; and the manifest's declared closure count is
//! cross-checked against the real member.

use docbrain_evidence::{verify_bundle, BundleBuilder, Verdict};

fn row_of(report: &docbrain_evidence::VerdictReport, row: u8) -> bool {
    report.findings.iter().any(|f| f.row == row)
}

fn finding_at(report: &docbrain_evidence::VerdictReport, row: u8, position: u64) -> bool {
    report.findings.iter().any(|f| f.row == row && f.position == Some(position))
}

// ---- row 13 via closure: out-of-range erasure, in-range target ----

#[test]
fn row_13_erasure_via_closure_is_valid_withheld_erased() {
    let bytes = BundleBuilder::new()
        .add_records(5)
        .with_content(1, b"hello world")
        .with_content(2, b"erase me via closure")
        .erase_via_closure(2)
        .build();
    let report = verify_bundle(&bytes);
    assert_eq!(report.verdict, Verdict::Valid, "{:?}", report.findings);
    assert!(finding_at(&report, 13, 2), "expected a row-13 finding at position 2, got: {:?}", report.findings);
    assert_eq!(report.counts.withheld_erased, 1);
    assert_eq!(report.counts.closure, 1, "the closure record must be counted");
    assert_eq!(report.counts.records, 5, "closure records must NOT inflate the in-range record count");
}

// ---- regression: rows 14/15 unaffected by the (now always-present) closure member ----

#[test]
fn row_14_still_bundle_incomplete_when_closure_is_empty() {
    let bytes = BundleBuilder::new()
        .add_records(5)
        .with_content(1, b"hello world")
        .with_content(2, b"dropped")
        .drop_erasure(2)
        .build();
    let report = verify_bundle(&bytes);
    assert_ne!(report.verdict, Verdict::Valid, "{:?}", report.findings);
    assert!(row_of(&report, 14), "expected row 14, got: {:?}", report.findings);
    assert!(!row_of(&report, 13), "must not be misread as an honest closure erasure");
}

#[test]
fn row_15_still_erasure_inconsistent_with_empty_closure_present() {
    let bytes = BundleBuilder::new()
        .add_records(5)
        .with_content(1, b"hello world")
        .with_content(2, b"kept despite erasure")
        .keep_content_despite_erasure(2)
        .build();
    let report = verify_bundle(&bytes);
    assert_ne!(report.verdict, Verdict::Valid, "{:?}", report.findings);
    assert!(row_of(&report, 15), "expected row 15, got: {:?}", report.findings);
}

// ---- row 22 (F2): dangling erasure target, in-range ----

#[test]
fn row_22_dangling_erasure_target_in_range_journal_is_malformed() {
    let bytes = BundleBuilder::new()
        .add_records(5)
        .dangling_erasure_target(999)
        .build();
    let report = verify_bundle(&bytes);
    assert_ne!(report.verdict, Verdict::Valid, "{:?}", report.findings);
    assert!(row_of(&report, 22), "expected row 22 for a dangling in-range erasure target, got: {:?}", report.findings);
}

// ---- controller fix round 1 (2026-08-25): in-range erasure targeting a
// record BELOW the export window's start is BENIGN, not row 22 ----
//
// The mainstream GDPR pattern: erase old content now, later export only a
// recent compliance window. The erasure event itself is in-range (it just
// happened); its long-erased target predates the window entirely. This
// must read VALID with no finding at all for the erasure record — a
// false-CANNOT_VERIFY here would be a cry-wolf regression against every
// honest windowed export that has ever erased anything.

#[test]
fn in_range_erasure_targeting_a_pre_window_record_is_valid_not_row_22() {
    let bytes = BundleBuilder::new()
        .add_records(5)
        .with_content(1, b"old content, erased long ago")
        .erase(1)
        // The erasure record lands at position 6 (after the 5 real
        // records). Window (5, 6] carries ONLY the erasure record itself —
        // position 1 (its target) is entirely outside this export.
        .export_window_start(5)
        .build();
    let report = verify_bundle(&bytes);
    assert_eq!(report.verdict, Verdict::Valid, "{:?}", report.findings);
    assert!(report.findings.is_empty(), "a pre-window erasure target must be BENIGN, no finding at all: {:?}", report.findings);
    assert_eq!(report.counts.records, 1, "the window carries only the erasure record itself");
    // Not withheld either — position 1's content isn't part of THIS
    // bundle at all, so there's nothing here to mark withheld-erased.
    assert_eq!(report.counts.withheld_erased, 0);
}

/// Same pre-window-target property, but for a NON-erasure-shaped garbage
/// target value: `target == 0` must still be row 22 even when
/// `start_position` is also 0 (the ordinary full-range-from-genesis case)
/// — position 0 is the virtual genesis anchor, never a real record,
/// regardless of window placement.
#[test]
fn in_range_erasure_targeting_position_zero_is_still_row_22() {
    let bytes = BundleBuilder::new().add_records(5).dangling_erasure_target(0).build();
    let report = verify_bundle(&bytes);
    assert_ne!(report.verdict, Verdict::Valid, "{:?}", report.findings);
    assert!(row_of(&report, 22), "target 0 must always be malformed, got: {:?}", report.findings);
}

// ---- row 22 (F2): dangling erasure target, via closure ----

#[test]
fn row_22_dangling_erasure_target_in_closure_is_malformed() {
    let bytes = BundleBuilder::new()
        .add_records(5)
        .dangling_closure_erasure_target(999)
        .build();
    let report = verify_bundle(&bytes);
    assert_ne!(report.verdict, Verdict::Valid, "{:?}", report.findings);
    assert!(
        row_of(&report, 22),
        "expected row 22 for a dangling closure erasure target, got: {:?}",
        report.findings
    );
}

// ---- structural violation: closure record's own position is NOT strictly outside the range ----

#[test]
fn row_22_closure_record_position_inside_range_is_malformed() {
    let bytes = BundleBuilder::new()
        .add_records(5)
        .with_content(1, b"hello world")
        .erase_via_closure(1)
        // Position 3 is well within the exported range [0, 5] — a closure
        // record claiming that position is structurally invalid; it should
        // have been an ordinary epoch entry, not a closure carry-forward.
        .closure_record_position_override(3)
        .build();
    let report = verify_bundle(&bytes);
    assert_ne!(report.verdict, Verdict::Valid, "{:?}", report.findings);
    assert!(
        row_of(&report, 22),
        "expected row 22 for a closure record whose position is not strictly outside the range, got: {:?}",
        report.findings
    );
}

// ---- structural violation: closure.jsonl may only carry erasure-kind records ----

#[test]
fn row_22_closure_record_with_non_erasure_kind_is_malformed() {
    let bytes = BundleBuilder::new()
        .add_records(5)
        .with_content(1, b"hello world")
        .erase_via_closure(1)
        .closure_record_wrong_kind()
        .build();
    let report = verify_bundle(&bytes);
    assert_ne!(report.verdict, Verdict::Valid, "{:?}", report.findings);
    assert!(
        row_of(&report, 22),
        "expected row 22 for a non-erasure-kind closure record, got: {:?}",
        report.findings
    );
}

// ---- test-plan case B4: a forged (bad-signature) closure record never produces a clean withheld-erased ----

#[test]
fn tampered_closure_signature_never_produces_a_clean_withheld_erased() {
    let bytes = BundleBuilder::new()
        .add_records(5)
        .with_content(1, b"hello world")
        .with_content(2, b"erase me via closure")
        .erase_via_closure(2)
        .tamper_closure_record()
        .build();
    let report = verify_bundle(&bytes);
    assert_ne!(
        report.verdict,
        Verdict::Valid,
        "a forged closure erasure record must never verify VALID, got: {:?}",
        report.findings
    );
    assert!(
        row_of(&report, 2),
        "expected a row-2 (tampered signature) finding for the forged closure record, got: {:?}",
        report.findings
    );
    assert!(
        !finding_at(&report, 13, 2),
        "the forged closure record must never produce a clean withheld-erased for its target, got: {:?}",
        report.findings
    );
}

// ---- closure-count cross-check ----

#[test]
fn closure_count_cross_check_mismatch_is_a_scope_finding() {
    let bytes = BundleBuilder::new()
        .add_records(5)
        .with_content(1, b"hello world")
        .with_content(2, b"erase me via closure")
        .erase_via_closure(2)
        .mismatched_manifest_closure_count()
        .build();
    let report = verify_bundle(&bytes);
    assert_eq!(report.verdict, Verdict::Tampered, "{:?}", report.findings);
    // Precise, not just "some row 10 exists": withheld_erased must be
    // independently correct (the closure erasure IS honored, row 13) —
    // isolating that the ONLY scope disagreement is the closure count
    // itself, not a knock-on withheld-count mismatch from closure never
    // being read at all.
    assert_eq!(report.counts.withheld_erased, 1, "{:?}", report.findings);
    assert!(finding_at(&report, 13, 2), "the closure erasure must still be honored, got: {:?}", report.findings);
    assert!(
        report.findings.iter().any(|f| f.row == 10 && f.detail.contains("closure")),
        "expected a row-10 finding specifically about the closure count, got: {:?}",
        report.findings
    );
}
