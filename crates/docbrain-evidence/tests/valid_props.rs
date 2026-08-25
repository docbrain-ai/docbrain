// SPDX-License-Identifier: MIT
//! Valid-bundle property gate (Task 16): the false-TAMPERED gate. Where
//! `mutation.rs` proves no TAMPERED bundle reads VALID, this proves the other
//! direction — an HONEST bundle, in ANY legal shape, NEVER reads non-VALID.
//!
//! proptest generates random-but-legal recipes across the whole honest shape
//! space — record counts, content payloads, honest erasures (in-range and via
//! closure), key rotations, linked anchors (witness and TSA-token), windowed
//! re-exports, and unusual-but-valid RFC-3339 checkpoint times — builds each
//! with `BundleBuilder`, and asserts `verify_bundle(...).verdict == Valid`.
//!
//! Every knob here is a combination the pipeline is designed to accept:
//! informational rows (13 withheld-erased, 23 anchor time-claim, 24 clock
//! anomaly, 25 trivial range) are explicitly VALID-compatible, so they do not
//! block. If proptest ever shrinks to a legal recipe that verifies non-VALID,
//! that is a real false-TAMPERED finding to escalate — the generator is NOT
//! constrained to dodge it. Illegal shapes (which `BundleBuilder` cannot even
//! construct honestly) are excluded precisely at construction, documented
//! inline, never by silencing a failure.

use docbrain_evidence::{verify_bundle, BundleBuilder, Verdict};
use proptest::prelude::*;

/// RFC-3339 forms proven (Task-16 probe) to parse identically in chrono and the
/// stdlib Python verifier — safe to sign into checkpoint `at` fields.
const VALID_TS_FORMS: &[&str] = &[
    "2026-01-01T00:00:00Z",
    "2026-01-01T00:00:00.000000500Z", // sub-microsecond
    "2026-01-01T00:00:00.123456789Z", // nanosecond
    "2026-01-01T00:00:60Z",           // leap second
    "2026-01-01T00:00:00+00:00",      // offset width
    "2026-01-01t02:00:00z",           // lowercase, later
];

#[derive(Debug, Clone)]
struct Recipe {
    n: u64,
    /// Per-position (1..=n) content flag; extra entries beyond `n` are ignored.
    content: Vec<bool>,
    /// Per-position erasure mode for positions that HAVE content: 0 = keep,
    /// 1 = honest in-range erase, 2 = honest erase via closure.
    erase_mode: Vec<u8>,
    rotation_at: Option<u64>,
    /// (is_witness, at_end).
    anchor: Option<(bool, bool)>,
    window: bool,
    /// Indices into `VALID_TS_FORMS` for (start, end) checkpoint times.
    ckpt_times: Option<(usize, usize)>,
}

fn recipe_strategy() -> impl Strategy<Value = Recipe> {
    (0u64..=8).prop_flat_map(|n| {
        let len = n as usize;
        (
            Just(n),
            prop::collection::vec(any::<bool>(), len),
            prop::collection::vec(0u8..3, len),
            prop::option::of(1u64..=n.max(1)),
            prop::option::of((any::<bool>(), any::<bool>())),
            any::<bool>(),
            prop::option::of((0usize..VALID_TS_FORMS.len(), 0usize..VALID_TS_FORMS.len())),
        )
            .prop_map(
                move |(n, content, erase_mode, rotation_at, anchor, window, ckpt_times)| Recipe {
                    n,
                    content,
                    erase_mode,
                    rotation_at,
                    anchor,
                    window,
                    ckpt_times,
                },
            )
    })
}

/// Translates a recipe into a legal honest bundle. Every constraint here is a
/// real legality boundary of an honest export (not a dodge): content precedes
/// its erasure; erasure and windowing are not combined (their count interaction
/// is a separate, already-tested surface); rotation and window positions stay
/// in range.
fn build(recipe: &Recipe) -> Vec<u8> {
    let n = recipe.n;
    let mut b = BundleBuilder::new();

    // Rotation only makes sense inside the journal.
    if n >= 1 {
        if let Some(r) = recipe.rotation_at {
            let r = r.clamp(1, n);
            b = b.with_rotation(r);
        }
    }
    b = b.add_records(n);

    // Content, then honest erasure of content-bearing positions.
    let mut erased_any = false;
    for pos in 1..=n {
        let idx = (pos - 1) as usize;
        if recipe.content.get(idx).copied().unwrap_or(false) {
            let payload = format!("valid-props-content-{pos}");
            b = b.with_content(pos, payload.as_bytes());
            match recipe.erase_mode.get(idx).copied().unwrap_or(0) {
                1 => {
                    b = b.erase(pos);
                    erased_any = true;
                }
                2 => {
                    b = b.erase_via_closure(pos);
                    erased_any = true;
                }
                _ => {}
            }
        }
    }

    // Linked anchor on a real checkpoint (0 = start, n = end).
    if let Some((is_witness, at_end)) = recipe.anchor {
        let cp = if at_end && n >= 1 { n } else { 0 };
        b = if is_witness {
            b.with_anchor_witness(cp)
        } else {
            b.with_anchor_token(cp, "2027-06-01T12:00:00Z")
        };
    }

    // Windowed re-export: only without erasure (their count interaction is a
    // separate tested surface), and only with room for a mid-range start.
    if recipe.window && n >= 2 && !erased_any {
        b = b.export_window_start(1);
    }

    // Unusual-but-valid signed checkpoint times.
    if let Some((s, e)) = recipe.ckpt_times {
        b = b.with_checkpoint_times(VALID_TS_FORMS[s], VALID_TS_FORMS[e]);
    }

    b.build()
}

proptest! {
    // Case count env-tunable; default 256 is plenty for the bounded shape space
    // and runs in well under a second (verify_bundle is in-process).
    #![proptest_config(ProptestConfig {
        cases: std::env::var("EVIDENCE_PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(256),
        // Integration-test crates have no lib.rs/main.rs for proptest's default
        // SourceParallel persistence to anchor to (it warns and skips), so point
        // it at the crate's committed regressions dir — keeping a failing case
        // replayable. cwd during `cargo test` is the crate root.
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::Direct(
                "proptest-regressions/valid_props.txt",
            ),
        )),
        ..ProptestConfig::default()
    })]

    #[test]
    fn every_legal_honest_bundle_verifies_valid(recipe in recipe_strategy()) {
        let bytes = build(&recipe);
        let report = verify_bundle(&bytes);
        prop_assert_eq!(
            report.verdict,
            Verdict::Valid,
            "false-TAMPERED: a legal honest recipe verified non-VALID \
             (verdict {:?}, dominant row {} [{}]): {:?}",
            report.verdict,
            report.dominant.row,
            report.dominant.code,
            recipe
        );
    }
}
