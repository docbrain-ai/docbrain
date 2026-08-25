// SPDX-License-Identifier: MIT
//! Throwaway fixture emitter for the Rust<->Python parity check (Task 14,
//! `tests-evidence/parity.sh`). Writes a representative — NOT exhaustive; the
//! full 26-row golden corpus + CI is Task 15 — set of genuine `.dbev` bundles
//! built via `BundleBuilder`, one per distinct verdict/dominant-code mechanism,
//! so both verifiers can be run over real exports and their verdicts diffed.
//!
//!   cargo run -q -p docbrain-evidence --example emit_fixtures -- <out_dir>

use docbrain_evidence::BundleBuilder;
use std::path::Path;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "tests-evidence/fixtures".to_string());
    let dir = Path::new(&out);
    std::fs::create_dir_all(dir).expect("create fixtures dir");

    // (name, bundle bytes). Name encodes the expected verdict + row purely for
    // human readability; the parity harness never trusts the name — it diffs
    // the two verifiers' actual output.
    let fixtures: Vec<(&str, Vec<u8>)> = vec![
        // ---- VALID ----
        ("valid_row1_happy", BundleBuilder::new().add_records(5).build()),
        (
            "valid_rotation_content",
            BundleBuilder::new().with_rotation(3).add_records(5).with_content(1, b"honest content").build(),
        ),
        ("valid_row25_trivial_range", BundleBuilder::new().build()),
        (
            "valid_row13_withheld_erased",
            BundleBuilder::new().add_records(5).with_content(2, b"erase me honestly").erase(2).build(),
        ),
        (
            "valid_row24_clock_anomaly",
            BundleBuilder::new().add_records(5).backwards_checkpoint_clock().build(),
        ),
        (
            "valid_row23_time_claim_falsified",
            BundleBuilder::new().add_records(5).with_anchor_token(5, "2026-01-01T00:30:00Z").build(),
        ),
        (
            "valid_row18_malformed_anchor",
            BundleBuilder::new().add_records(5).malformed_anchor(5).build(),
        ),
        (
            "valid_row19_unlinked_anchor",
            BundleBuilder::new().add_records(5).unlinked_anchor(9_999_999).build(),
        ),
        (
            "valid_row13_closure_erasure",
            BundleBuilder::new().add_records(5).with_content(2, b"closure-erase").erase_via_closure(2).build(),
        ),
        // ---- TAMPERED ----
        ("tampered_row2_signature", BundleBuilder::new().add_records(5).tamper_record(3).build()),
        ("tampered_row2_last_record", BundleBuilder::new().add_records(5).tamper_record(5).build()),
        ("tampered_row4_forged_prev_head", BundleBuilder::new().add_records(5).forge_prev_head(2).build()),
        (
            "tampered_row3_key_epoch",
            BundleBuilder::new().with_rotation(3).add_records(5).forge_position(4, 1).build(),
        ),
        (
            "tampered_row5_invalid_rotation",
            BundleBuilder::new().with_rotation(3).corrupt_rotation_signer(3).add_records(5).build(),
        ),
        (
            "tampered_row6_unauthorized_control",
            BundleBuilder::new()
                .with_recovery_key()
                .add_records(5)
                .with_compromise(100, "2026-06-01T00:00:00Z")
                .corrupt_compromise_signer()
                .build(),
        ),
        (
            "tampered_row7_post_compromise",
            BundleBuilder::new()
                .with_recovery_key()
                .add_records(5)
                .with_compromise(3, "2026-06-01T00:00:00Z")
                .build(),
        ),
        (
            "tampered_row10_mismatched_counts",
            BundleBuilder::new().add_records(5).mismatched_manifest_counts().build(),
        ),
        (
            "tampered_row10_mismatched_export_checkpoint",
            BundleBuilder::new().add_records(5).mismatched_manifest_export_checkpoint().build(),
        ),
        ("tampered_row11_manifest", BundleBuilder::new().add_records(5).tamper_manifest().build()),
        (
            "tampered_row12_content",
            BundleBuilder::new().add_records(5).with_content(2, b"original content bytes").tamper_content(2).build(),
        ),
        // ---- CANNOT_VERIFY ----
        (
            "cannotverify_row9_indeterminate",
            BundleBuilder::new()
                .with_recovery_key()
                .add_records(5)
                .with_compromise(100, "2026-06-01T00:00:00Z")
                .build(),
        ),
        (
            "cannotverify_row14_incomplete",
            BundleBuilder::new().add_records(5).with_content(2, b"missing, unexplained").drop_erasure(2).build(),
        ),
        (
            "cannotverify_row15_erasure_inconsistent",
            BundleBuilder::new().add_records(5).with_content(2, b"resurrected content").keep_content_despite_erasure(2).build(),
        ),
        (
            "cannotverify_row16_unknown_key",
            BundleBuilder::new().add_records(5).unknown_key_record(3).build(),
        ),
        (
            "cannotverify_row17_multi_sig",
            BundleBuilder::new().add_records(5).multi_sig_record(3).build(),
        ),
        (
            "cannotverify_row21_unlisted_member",
            BundleBuilder::new().add_records(5).unlisted_container_member().build(),
        ),
        (
            "cannotverify_row21_duplicate_member",
            BundleBuilder::new().add_records(5).duplicate_member("checkpoints.jsonl").build(),
        ),
        (
            "cannotverify_row22_wrong_payload_type",
            BundleBuilder::new().add_records(5).wrong_payload_type_record(3).build(),
        ),
        // ---- combo: TAMPERED dominant + informational anchor finding ----
        (
            "combo_tamper_record_and_malformed_anchor",
            BundleBuilder::new().add_records(5).tamper_record(3).malformed_anchor(5).build(),
        ),
        // ---- non-bundle inputs (fail-closed, never VALID) ----
        ("nonbundle_empty", Vec::new()),
        ("nonbundle_garbage", b"this is not a zip file at all, just noise 0xdeadbeef".to_vec()),
    ];

    for (name, bytes) in &fixtures {
        let path = dir.join(format!("{name}.dbev"));
        std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    }
    println!("wrote {} fixtures to {}", fixtures.len(), dir.display());
}
