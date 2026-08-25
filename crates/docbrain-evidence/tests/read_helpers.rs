// SPDX-License-Identifier: MIT
//! Integration tests for the additive read helpers `read_records` and
//! `chain_heads_for_bundle` (Task 13) — the thin, offline surfaces the CLI's
//! `evidence why`/`tables`/`--against` build on WITHOUT reparsing `.dbev`
//! bytes themselves. Kept out of `src/verify.rs` deliberately: the
//! `single_success_exit` meta-test counts `Verdict::Valid` occurrences in
//! that source file and requires exactly one (the pipeline's sole success
//! exit), so verdict assertions belong here, not inline.

use docbrain_evidence::{chain_heads_for_bundle, read_records, verify_bundle, BundleBuilder, Verdict};

#[test]
fn read_records_returns_every_epoch_record_in_order() {
    let bytes = BundleBuilder::new().add_records(5).build();
    assert_eq!(verify_bundle(&bytes).verdict, Verdict::Valid);
    let records = read_records(&bytes).expect("records must read");
    assert_eq!(records.len(), 5);
    for (i, r) in records.iter().enumerate() {
        assert_eq!(r.position, i as u64 + 1);
    }
    // The body the builder writes is the closed-schema `{"seq": position}`.
    assert_eq!(records[0].body, serde_json::json!({"seq": 1}));
}

#[test]
fn read_records_of_a_zero_record_bundle_is_empty() {
    let bytes = BundleBuilder::new().build();
    assert_eq!(verify_bundle(&bytes).verdict, Verdict::Valid);
    assert!(read_records(&bytes).expect("must read").is_empty());
}

#[test]
fn chain_heads_for_bundle_prefixes_the_genesis_anchor_then_every_position() {
    let bytes = BundleBuilder::new().add_records(4).build();
    assert_eq!(verify_bundle(&bytes).verdict, Verdict::Valid);
    let bundle = chain_heads_for_bundle(&bytes).expect("heads must resolve");
    // anchor (0, GENESIS_PREV) + 4 walked positions.
    assert_eq!(bundle.heads.len(), 5);
    assert_eq!(bundle.heads[0], (0, [0u8; 32]));
    for (i, (pos, _)) in bundle.heads.iter().enumerate().skip(1) {
        assert_eq!(*pos, i as u64);
    }
}

#[test]
fn two_identical_journals_agree_on_every_overlapping_head_and_identity() {
    // Deterministic builder → byte-identical bundles → identical heads AND
    // identical genesis identity.
    let a = chain_heads_for_bundle(&BundleBuilder::new().add_records(6).build()).unwrap();
    let b = chain_heads_for_bundle(&BundleBuilder::new().add_records(6).build()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn two_exports_of_the_same_journal_share_genesis_identity() {
    // Different lengths, SAME default genesis key → same journal identity.
    let a = chain_heads_for_bundle(&BundleBuilder::new().add_records(5).build()).unwrap();
    let b = chain_heads_for_bundle(&BundleBuilder::new().add_records(3).build()).unwrap();
    assert_eq!(a.genesis_identity, b.genesis_identity);
}

#[test]
fn distinct_genesis_keys_yield_distinct_identity_while_heads_still_fork() {
    // Same records, DIFFERENT genesis key → both VALID, share position
    // numbers with DIFFERENT heads (the head-level fork precondition holds),
    // but distinct journal identity. This is precisely the pair the CLI's
    // --against gate must call "different journals", not a fork: the identity
    // differs AND (proving the gate is load-bearing) the heads themselves
    // WOULD fork at position 1 without it.
    let a_bytes = BundleBuilder::new().add_records(4).build();
    let b_bytes = BundleBuilder::new().with_genesis_key_seed(200).add_records(4).build();
    assert_eq!(verify_bundle(&a_bytes).verdict, Verdict::Valid);
    assert_eq!(verify_bundle(&b_bytes).verdict, Verdict::Valid);
    let a = chain_heads_for_bundle(&a_bytes).unwrap();
    let b = chain_heads_for_bundle(&b_bytes).unwrap();
    assert_ne!(a.genesis_identity, b.genesis_identity, "distinct genesis keys = distinct journals");
    let head_at = |v: &[(u64, [u8; 32])], p: u64| v.iter().find(|(q, _)| *q == p).map(|(_, h)| *h);
    assert!(head_at(&a.heads, 1).is_some() && head_at(&b.heads, 1).is_some());
    assert_ne!(
        head_at(&a.heads, 1),
        head_at(&b.heads, 1),
        "without the identity gate this pair would FORK at position 1"
    );
}

#[test]
fn a_prefix_export_is_head_compatible_with_the_longer_one() {
    // add_records(3) is a byte-prefix of add_records(5): their shared
    // positions 1..3 must carry identical running heads (the "consistent" /
    // prefix-compatible case --against reports).
    let short = chain_heads_for_bundle(&BundleBuilder::new().add_records(3).build()).unwrap();
    let long = chain_heads_for_bundle(&BundleBuilder::new().add_records(5).build()).unwrap();
    for (pos, head) in &short.heads {
        if let Some((_, other)) = long.heads.iter().find(|(p, _)| p == pos) {
            assert_eq!(head, other, "head at shared position {pos} must match");
        }
    }
}

#[test]
fn differing_content_forks_the_head_from_the_diverging_position() {
    // Two independently-Valid bundles of equal length whose record 2 content
    // differs: identical head at position 1, divergent from position 2 on —
    // the cryptographic fork --against must detect even though BOTH bundles
    // verify Valid on their own. (Same genesis key, so the identity gate lets
    // this through to the head comparison — a genuine intra-journal fork.)
    let a_bytes = BundleBuilder::new().add_records(4).with_content(2, b"alpha").build();
    let b_bytes = BundleBuilder::new().add_records(4).with_content(2, b"beta").build();
    assert_eq!(verify_bundle(&a_bytes).verdict, Verdict::Valid);
    assert_eq!(verify_bundle(&b_bytes).verdict, Verdict::Valid);
    let a = chain_heads_for_bundle(&a_bytes).unwrap();
    let b = chain_heads_for_bundle(&b_bytes).unwrap();
    assert_eq!(a.genesis_identity, b.genesis_identity, "same genesis → same journal");
    let head_at = |v: &[(u64, [u8; 32])], p: u64| v.iter().find(|(q, _)| *q == p).map(|(_, h)| *h);
    assert_eq!(head_at(&a.heads, 1), head_at(&b.heads, 1), "position 1 identical");
    assert_ne!(head_at(&a.heads, 2), head_at(&b.heads, 2), "position 2 forks");
    assert_ne!(head_at(&a.heads, 4), head_at(&b.heads, 4), "and stays forked to the tip");
}
