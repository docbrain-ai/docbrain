// SPDX-License-Identifier: MIT
//! Golden-corpus generator (Task 15). Writes the FROZEN, COMMITTED ground
//! truth the cross-verifier parity CI checks forever:
//!
//!   tests-evidence/corpus/valid.dbev            (taxonomy row 1)
//!   tests-evidence/corpus/row-NN-<code>.dbev    (rows 2-25, minus deferred 8/20)
//!   tests-evidence/corpus/expected.json         (OBSERVED verdict per file)
//!
//!   cargo run -q -p docbrain-evidence --example gen_corpus -- tests-evidence/corpus
//!
//! ## Regeneration is a MANUAL, REVIEWED act
//!
//! The `.dbev` files and `expected.json` are committed. CI never runs this
//! generator — it verifies the committed bytes with BOTH verifiers (Rust
//! `docbrain-verify` + `tools/verify_dbev.py`) against the frozen
//! `expected.json`. Rerun this ONLY to deliberately re-cut the corpus, and
//! review the `expected.json` diff by hand: a changed verdict is either an
//! intended taxonomy change or a regression.
//!
//! ## Why this generator can't lie (the three-way lock)
//!
//! Every entry carries a HARDCODED *intended* `(row, code, verdict)` — my
//! reading of the taxonomy, fixed independently of any verifier run. The file
//! is NAMED from the intended values. `expected.json` is written from the
//! OBSERVED `verify_bundle` output. Before writing anything, this generator
//! asserts observed == intended for every entry and EXITS NONZERO on any
//! mismatch, printing the full intended-vs-observed table. So a mislabeled or
//! aspirational corpus can never be produced: either the observed verdict is
//! exactly the row the filename claims, or generation fails loudly. (This is
//! the design-doc "HARD DISCIPLINE": expected.json is observed, never
//! aspirational; a surprising observation is a finding to investigate, not a
//! file to relabel.)
//!
//! Deferred rows (8, 20, 26) are NOT reachable through the standard offline
//! verifiers the parity harness runs; they are recorded as explicit deferred
//! entries in `expected.json` with a reason, never as fabricated `.dbev`.
//! See the Task 15 brief's controller ruling and the corpus README.

use docbrain_evidence::{verify_bundle, BundleBuilder};
use serde_json::{json, Map, Value};
use std::path::Path;
use std::process::ExitCode;

/// One corpus entry: the taxonomy row it exercises, the dominant-finding code
/// and verdict I INTEND it to produce (hardcoded from the taxonomy, not from a
/// verifier run), and the recipe that builds its bytes.
struct Entry {
    intended_row: u8,
    intended_code: &'static str,
    intended_verdict: &'static str,
    build: fn() -> Vec<u8>,
}

impl Entry {
    /// The frozen filename, derived from the INTENDED row/code. Row 1 (the
    /// all-checks-pass happy path) is the specially-named `valid.dbev`; every
    /// other row is `row-NN-<code>.dbev`.
    fn file_name(&self) -> String {
        if self.intended_row == 1 {
            "valid.dbev".to_string()
        } else {
            format!("row-{:02}-{}.dbev", self.intended_row, self.intended_code)
        }
    }
}

fn corpus() -> Vec<Entry> {
    vec![
        // ---- VALID (rows 1, 13, 18, 19, 23, 24, 25) ----
        Entry {
            intended_row: 1,
            intended_code: "valid",
            intended_verdict: "VALID",
            build: || BundleBuilder::new().add_records(5).build(),
        },
        Entry {
            intended_row: 13,
            intended_code: "withheld-erased",
            intended_verdict: "VALID",
            build: || {
                BundleBuilder::new()
                    .add_records(5)
                    .with_content(2, b"erase me honestly")
                    .erase(2)
                    .build()
            },
        },
        Entry {
            intended_row: 18,
            intended_code: "anchor-invalid",
            intended_verdict: "VALID",
            build: || BundleBuilder::new().add_records(5).malformed_anchor(5).build(),
        },
        Entry {
            intended_row: 19,
            intended_code: "anchor-unlinked",
            intended_verdict: "VALID",
            build: || BundleBuilder::new().add_records(5).unlinked_anchor(9_999_999).build(),
        },
        Entry {
            intended_row: 23,
            intended_code: "time-claim-falsified",
            intended_verdict: "VALID",
            build: || {
                BundleBuilder::new()
                    .add_records(5)
                    .with_anchor_token(5, "2026-01-01T00:30:00Z")
                    .build()
            },
        },
        Entry {
            intended_row: 24,
            intended_code: "clock-anomaly",
            intended_verdict: "VALID",
            build: || BundleBuilder::new().add_records(5).backwards_checkpoint_clock().build(),
        },
        Entry {
            intended_row: 25,
            intended_code: "trivial-range",
            intended_verdict: "VALID",
            build: || BundleBuilder::new().build(),
        },
        // ---- TAMPERED (rows 2, 3, 4, 5, 6, 7, 10, 11, 12) ----
        Entry {
            intended_row: 2,
            intended_code: "tampered-signature",
            intended_verdict: "TAMPERED",
            build: || BundleBuilder::new().add_records(5).tamper_record(3).build(),
        },
        Entry {
            intended_row: 3,
            intended_code: "tampered-key-epoch",
            intended_verdict: "TAMPERED",
            build: || BundleBuilder::new().with_rotation(3).add_records(5).forge_position(4, 1).build(),
        },
        Entry {
            intended_row: 4,
            intended_code: "tampered-chain",
            intended_verdict: "TAMPERED",
            build: || BundleBuilder::new().add_records(5).forge_prev_head(2).build(),
        },
        Entry {
            intended_row: 5,
            intended_code: "tampered-invalid-rotation",
            intended_verdict: "TAMPERED",
            build: || {
                BundleBuilder::new()
                    .with_rotation(3)
                    .corrupt_rotation_signer(3)
                    .add_records(5)
                    .build()
            },
        },
        Entry {
            intended_row: 6,
            intended_code: "tampered-unauthorized-control-record",
            intended_verdict: "TAMPERED",
            build: || {
                BundleBuilder::new()
                    .with_recovery_key()
                    .add_records(5)
                    .with_compromise(100, "2026-06-01T00:00:00Z")
                    .corrupt_compromise_signer()
                    .build()
            },
        },
        Entry {
            intended_row: 7,
            intended_code: "tampered-post-compromise-position",
            intended_verdict: "TAMPERED",
            build: || {
                BundleBuilder::new()
                    .with_recovery_key()
                    .add_records(5)
                    .with_compromise(3, "2026-06-01T00:00:00Z")
                    .build()
            },
        },
        Entry {
            intended_row: 10,
            intended_code: "tampered-scope",
            intended_verdict: "TAMPERED",
            build: || BundleBuilder::new().add_records(5).mismatched_manifest_counts().build(),
        },
        Entry {
            intended_row: 11,
            intended_code: "tampered-manifest",
            intended_verdict: "TAMPERED",
            build: || BundleBuilder::new().add_records(5).tamper_manifest().build(),
        },
        Entry {
            intended_row: 12,
            intended_code: "tampered-content",
            intended_verdict: "TAMPERED",
            build: || {
                BundleBuilder::new()
                    .add_records(5)
                    .with_content(2, b"original content bytes")
                    .tamper_content(2)
                    .build()
            },
        },
        // ---- CANNOT_VERIFY (rows 9, 14, 15, 16, 17, 21, 22) ----
        Entry {
            intended_row: 9,
            intended_code: "cannot-verify-compromise-window-indeterminate",
            intended_verdict: "CANNOT_VERIFY",
            build: || {
                BundleBuilder::new()
                    .with_recovery_key()
                    .add_records(5)
                    .with_compromise(100, "2026-06-01T00:00:00Z")
                    .build()
            },
        },
        Entry {
            intended_row: 14,
            intended_code: "cannot-verify-bundle-incomplete",
            intended_verdict: "CANNOT_VERIFY",
            build: || {
                BundleBuilder::new()
                    .add_records(5)
                    .with_content(2, b"missing, unexplained")
                    .drop_erasure(2)
                    .build()
            },
        },
        Entry {
            intended_row: 15,
            intended_code: "cannot-verify-erasure-inconsistent",
            intended_verdict: "CANNOT_VERIFY",
            build: || {
                BundleBuilder::new()
                    .add_records(5)
                    .with_content(2, b"resurrected content")
                    .keep_content_despite_erasure(2)
                    .build()
            },
        },
        Entry {
            intended_row: 16,
            intended_code: "cannot-verify-unknown-key",
            intended_verdict: "CANNOT_VERIFY",
            build: || BundleBuilder::new().add_records(5).unknown_key_record(3).build(),
        },
        Entry {
            intended_row: 17,
            intended_code: "cannot-verify-unsupported-format",
            intended_verdict: "CANNOT_VERIFY",
            build: || BundleBuilder::new().add_records(5).multi_sig_record(3).build(),
        },
        Entry {
            intended_row: 21,
            intended_code: "cannot-verify-container-profile",
            intended_verdict: "CANNOT_VERIFY",
            build: || BundleBuilder::new().add_records(5).unlisted_container_member().build(),
        },
        Entry {
            intended_row: 22,
            intended_code: "cannot-verify-malformed",
            intended_verdict: "CANNOT_VERIFY",
            build: || BundleBuilder::new().add_records(5).wrong_payload_type_record(3).build(),
        },
    ]
}

/// The deferred-entry block of `expected.json` — rows the standard offline
/// verifiers the parity harness runs can never produce in v1. Recorded
/// honestly with a reason and a Task-17 pointer, never as a fabricated
/// `.dbev`. (Task 15 brief controller ruling.)
fn deferred() -> Value {
    json!([
        {
            "row": 8,
            "code": "valid-pre-claim",
            "reachable": false,
            "deferred_to": "Task 17",
            "reason": "requires verify_bundle_with_witness / tier>=2 anchor; not reachable via the standard offline verifiers the parity harness runs (an intended-row-8 bundle verifies as row 9 there)"
        },
        {
            "row": 20,
            "code": "anchor-stale",
            "reachable": false,
            "deferred_to": "Task 17",
            "reason": "anchor-stale not implemented until anchors v1 (no CODE_ANCHOR_STALE; verify.rs never validates an anchor to tier>=2)"
        },
        {
            "row": 26,
            "code": "unmapped-state",
            "reachable": false,
            "reason": "closed-world default; structurally unreachable through verify_bundle (every finding maps to rows 1-25 by exhaustive matching). Covered by unit test verdict::tests::unmapped_row_becomes_row_26, not a container fixture."
        }
    ])
}

fn main() -> ExitCode {
    let out = std::env::args().nth(1).unwrap_or_else(|| "tests-evidence/corpus".to_string());
    let dir = Path::new(&out);
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("error: create corpus dir {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }

    let entries = corpus();
    let mut files_map = Map::new();
    let mut mismatches: Vec<String> = Vec::new();

    println!(
        "{:<6} {:<50} {:<14} {:<14} {}",
        "row", "code (intended)", "verdict(int)", "verdict(obs)", "row/code observed"
    );
    println!("{}", "-".repeat(120));

    for e in &entries {
        let bytes = (e.build)();
        let name = e.file_name();
        let path = dir.join(&name);
        if let Err(err) = std::fs::write(&path, &bytes) {
            eprintln!("error: write {}: {err}", path.display());
            return ExitCode::FAILURE;
        }

        // OBSERVE: run the real verifier over the bytes we just wrote.
        let report = verify_bundle(&bytes);
        let obs_verdict = report.verdict.as_str();
        let obs_row = report.dominant.row;
        let obs_code = report.dominant.code;

        let ok = obs_row == e.intended_row
            && obs_code == e.intended_code
            && obs_verdict == e.intended_verdict;
        let flag = if ok { "ok " } else { "!! " };
        println!(
            "{}{:<4} {:<50} {:<14} {:<14} row {} / {}",
            flag, e.intended_row, e.intended_code, e.intended_verdict, obs_verdict, obs_row, obs_code
        );
        if !ok {
            mismatches.push(format!(
                "{name}: intended (row {}, {}, {}) but OBSERVED (row {}, {}, {})",
                e.intended_row, e.intended_code, e.intended_verdict, obs_row, obs_code, obs_verdict
            ));
        }

        // expected.json is written from OBSERVED values (never intended).
        files_map.insert(
            name,
            json!({
                "verdict": obs_verdict,
                "dominant_row": obs_row,
                "dominant_code": obs_code,
            }),
        );
    }

    if !mismatches.is_empty() {
        eprintln!("\nGENERATION FAILED — observed verdict != intended for {} entry(ies):", mismatches.len());
        for m in &mismatches {
            eprintln!("  - {m}");
        }
        eprintln!(
            "\nThis is a HARD-DISCIPLINE stop: the corpus was NOT finalized. Investigate whether the\n\
             builder recipe or the intended taxonomy mapping is wrong — do NOT relabel the file to\n\
             match the surprise. (expected.json was left with the observed values written above so\n\
             the divergence is inspectable, but the process exits nonzero so nothing downstream\n\
             treats this run as authoritative.)"
        );
        // Still write expected.json (observed) so the mismatch is inspectable,
        // then fail.
        write_expected(dir, files_map);
        return ExitCode::FAILURE;
    }

    write_expected(dir, files_map);
    println!(
        "\nOK: {} corpus files written to {} (+ expected.json). Deferred rows: 8, 20, 26.",
        entries.len(),
        dir.display()
    );
    ExitCode::SUCCESS
}

fn write_expected(dir: &Path, files_map: Map<String, Value>) {
    // serde_json's Map (no preserve_order feature) sorts keys, so this output
    // is deterministic across runs — a clean git diff on any real change.
    let doc = json!({
        "_frozen": "GROUND TRUTH for the cross-verifier parity CI. Regeneration is a manual, reviewed act; CI never runs the generator. See tests-evidence/corpus/README.md.",
        "files": Value::Object(files_map),
        "deferred": deferred(),
    });
    let path = dir.join("expected.json");
    let text = serde_json::to_string_pretty(&doc).expect("serialize expected.json");
    std::fs::write(&path, format!("{text}\n")).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}
