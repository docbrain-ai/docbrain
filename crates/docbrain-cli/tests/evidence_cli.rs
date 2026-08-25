// SPDX-License-Identifier: MIT
//! End-to-end tests for the `evidence` CLI family and the standalone
//! `docbrain-verify` binary (Task 13). Bundles are built on disk with the MIT
//! crate's `BundleBuilder`; the compiled binaries are then run as
//! subprocesses so the assertions cover the REAL exit codes an auditor sees.
//!
//! The load-bearing invariant of this whole feature: the verdict-bearing
//! paths (`verify`, `--against`) must be perfect in BOTH directions — a false
//! VALID on a tampered bundle and a false TAMPERED/CANNOT_VERIFY on an honest
//! one are equally disqualifying. Every verdict/exit-code assertion below
//! exists to pin one of those.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use docbrain_evidence::BundleBuilder;

fn bin_verify() -> &'static str {
    env!("CARGO_BIN_EXE_docbrain-verify")
}
fn bin_cli() -> &'static str {
    env!("CARGO_BIN_EXE_docbrain-cli")
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh, unique temp dir for one test's bundle files.
fn scratch() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("dbev-cli-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write_bundle(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write bundle");
    path
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Run a binary offline: an empty HOME (so no `~/.docbrain/config.json`
/// exists) and the DocBrain env vars cleared. verify/why/tables have no
/// network code path at all; this proves it by construction.
fn run_offline(program: &str, args: &[&str], home: &Path) -> Run {
    let output = Command::new(program)
        .args(args)
        .env("HOME", home)
        .env_remove("DOCBRAIN_API_KEY")
        .env_remove("DOCBRAIN_SERVER_URL")
        .output()
        .expect("spawn binary");
    Run {
        code: output.status.code().expect("process exited normally (not by signal)"),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

// ---- bundle fixtures ----

fn valid_bundle() -> Vec<u8> {
    BundleBuilder::new().add_records(5).build()
}
fn tampered_bundle() -> Vec<u8> {
    BundleBuilder::new().add_records(5).tamper_record(3).build()
}
/// Content declared but withheld with NO erasure record → row 14
/// (bundle-incomplete) → CANNOT_VERIFY.
fn incomplete_bundle() -> Vec<u8> {
    BundleBuilder::new().add_records(5).with_content(2, b"x").drop_erasure(2).build()
}

// ═══════════════════════════════════════════════════════════════════════════
// standalone docbrain-verify: exit code IS the verdict
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn standalone_verify_valid_exits_0() {
    let dir = scratch();
    let b = write_bundle(&dir, "ok.dbev", &valid_bundle());
    let r = run_offline(bin_verify(), &[b.to_str().unwrap()], &dir);
    assert_eq!(r.code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stdout.contains("VALID"), "{}", r.stdout);
}

#[test]
fn standalone_verify_tampered_exits_1() {
    let dir = scratch();
    let b = write_bundle(&dir, "bad.dbev", &tampered_bundle());
    let r = run_offline(bin_verify(), &[b.to_str().unwrap()], &dir);
    assert_eq!(r.code, 1, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stdout.contains("TAMPERED"), "{}", r.stdout);
}

#[test]
fn standalone_verify_incomplete_exits_2() {
    let dir = scratch();
    let b = write_bundle(&dir, "incomplete.dbev", &incomplete_bundle());
    let r = run_offline(bin_verify(), &[b.to_str().unwrap()], &dir);
    assert_eq!(r.code, 2, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stdout.contains("CANNOT_VERIFY"), "{}", r.stdout);
}

#[test]
fn standalone_verify_missing_file_exits_3() {
    let dir = scratch();
    let r = run_offline(bin_verify(), &[dir.join("does-not-exist.dbev").to_str().unwrap()], &dir);
    assert_eq!(r.code, 3, "a missing file is a CLI error (3), never a verdict");
    assert!(r.stderr.contains("cannot read"), "{}", r.stderr);
}

#[test]
fn standalone_verify_json_is_wellformed_and_carries_the_verdict() {
    let dir = scratch();
    let b = write_bundle(&dir, "ok.dbev", &valid_bundle());
    let r = run_offline(bin_verify(), &[b.to_str().unwrap(), "--json"], &dir);
    assert_eq!(r.code, 0);
    let v: serde_json::Value = serde_json::from_str(&r.stdout).expect("valid JSON");
    assert_eq!(v["verdict"], "VALID");
    assert_eq!(v["exit_code"], 0);
}

// ═══════════════════════════════════════════════════════════════════════════
// docbrain evidence verify: same verdicts/exit codes as the standalone binary
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn evidence_verify_matches_standalone_exit_codes() {
    let dir = scratch();
    for (name, bytes, want) in [
        ("ok.dbev", valid_bundle(), 0),
        ("bad.dbev", tampered_bundle(), 1),
        ("incomplete.dbev", incomplete_bundle(), 2),
    ] {
        let b = write_bundle(&dir, name, &bytes);
        let r = run_offline(bin_cli(), &["evidence", "verify", b.to_str().unwrap()], &dir);
        assert_eq!(r.code, want, "{name}: stdout={} stderr={}", r.stdout, r.stderr);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// --against: consistent / FORK DETECTED / not-comparable
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn against_consistent_prefix_export_exits_with_primary_verdict() {
    let dir = scratch();
    // add_records(3) is a byte-prefix of add_records(5): overlapping
    // positions 1..3 agree → consistent. Primary (the 5-record bundle) is
    // VALID → exit 0.
    let primary = write_bundle(&dir, "full.dbev", &BundleBuilder::new().add_records(5).build());
    let earlier = write_bundle(&dir, "earlier.dbev", &BundleBuilder::new().add_records(3).build());
    let r = run_offline(
        bin_cli(),
        &["evidence", "verify", primary.to_str().unwrap(), "--against", earlier.to_str().unwrap()],
        &dir,
    );
    assert_eq!(r.code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stdout.contains("consistent"), "{}", r.stdout);
    // The honest caveat MUST be printed on a consistency claim.
    assert!(r.stdout.contains("retained the earlier bundle independently"), "{}", r.stdout);
}

#[test]
fn against_fork_forces_exit_1_even_though_both_bundles_are_individually_valid() {
    let dir = scratch();
    // Two INDEPENDENTLY-VALID bundles whose record 2 content differs: they
    // fork at position 2. This is the catastrophic-if-missed case — a false
    // "consistent" here would be a shipping blocker.
    let a = write_bundle(&dir, "a.dbev", &BundleBuilder::new().add_records(4).with_content(2, b"alpha").build());
    let b = write_bundle(&dir, "b.dbev", &BundleBuilder::new().add_records(4).with_content(2, b"beta").build());
    let r = run_offline(
        bin_cli(),
        &["evidence", "verify", a.to_str().unwrap(), "--against", b.to_str().unwrap()],
        &dir,
    );
    assert_eq!(r.code, 1, "a proven fork must force a failing exit: stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stdout.contains("FORK DETECTED"), "{}", r.stdout);
    assert!(r.stdout.contains("position 2"), "fork must localize to position 2: {}", r.stdout);
}

#[test]
fn against_fork_at_the_last_shared_position_is_still_detected() {
    // THE boundary case the running-head design exists for: records 1..3 are
    // byte-identical, ONLY record 4 (the last shared position) differs. A
    // naive comparison of each record's declared `prev_head` would see
    // prev_head@4 == head-after-3 == identical and falsely report
    // "consistent". Comparing the running head AFTER each position catches the
    // divergence at position 4. A miss here is a false VALID — the exact
    // catastrophe this feature must never commit.
    let dir = scratch();
    let a = write_bundle(&dir, "a.dbev", &BundleBuilder::new().add_records(4).with_content(4, b"x").build());
    let b = write_bundle(&dir, "b.dbev", &BundleBuilder::new().add_records(4).with_content(4, b"y").build());
    let r = run_offline(
        bin_cli(),
        &["evidence", "verify", a.to_str().unwrap(), "--against", b.to_str().unwrap()],
        &dir,
    );
    assert_eq!(r.code, 1, "fork at the last shared position must be caught: stdout={}", r.stdout);
    assert!(r.stdout.contains("FORK DETECTED"), "{}", r.stdout);
    assert!(r.stdout.contains("position 4"), "{}", r.stdout);
}

#[test]
fn against_two_unrelated_journals_are_different_journals_not_a_fork() {
    // REQUIRED (fix round 1): two bundles with DIFFERENT genesis signing keys
    // that share position numbers with DIFFERENT heads. Without the identity
    // gate this pair returns "FORK DETECTED, exit 1" — a false fraud
    // accusation for what is a plausible operator mistake (grabbed the wrong
    // earlier file). With the gate it must be "different journals /
    // not-comparable", exit stays the primary bundle's verdict (0), and it is
    // never a fork. The crate test
    // `distinct_genesis_keys_yield_distinct_identity_while_heads_still_fork`
    // proves the head-level fork precondition genuinely holds here, so this is
    // the gate doing the narrowing, not an absence of divergence.
    let dir = scratch();
    let a = write_bundle(&dir, "journal-x.dbev", &BundleBuilder::new().add_records(4).build());
    let b = write_bundle(
        &dir,
        "journal-y.dbev",
        &BundleBuilder::new().with_genesis_key_seed(200).add_records(4).build(),
    );
    let r = run_offline(
        bin_cli(),
        &["evidence", "verify", a.to_str().unwrap(), "--against", b.to_str().unwrap()],
        &dir,
    );
    assert_eq!(r.code, 0, "different journals must NOT force a fork exit: stdout={}", r.stdout);
    assert!(r.stdout.contains("different journal"), "{}", r.stdout);
    assert!(!r.stdout.contains("FORK DETECTED"), "must never accuse unrelated journals of a fork: {}", r.stdout);
    assert!(!r.stdout.contains("consistent"), "and must not claim consistency either: {}", r.stdout);
}

#[test]
fn against_identical_bundle_with_itself_is_consistent() {
    let dir = scratch();
    let b = write_bundle(&dir, "ok.dbev", &valid_bundle());
    let r = run_offline(
        bin_cli(),
        &["evidence", "verify", b.to_str().unwrap(), "--against", b.to_str().unwrap()],
        &dir,
    );
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("consistent"), "{}", r.stdout);
}

#[test]
fn against_two_zero_record_bundles_are_not_comparable_not_falsely_consistent() {
    // Two empty bundles share only the universal genesis anchor (position 0),
    // which carries no journal identity — so there is NO real overlap and no
    // consistency may be claimed.
    let dir = scratch();
    let a = write_bundle(&dir, "z1.dbev", &BundleBuilder::new().build());
    let b = write_bundle(&dir, "z2.dbev", &BundleBuilder::new().build());
    let r = run_offline(
        bin_cli(),
        &["evidence", "verify", a.to_str().unwrap(), "--against", b.to_str().unwrap()],
        &dir,
    );
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("not-comparable"), "must not claim consistency on the genesis anchor alone: {}", r.stdout);
}

#[test]
fn against_a_garbage_earlier_file_is_skipped_not_a_crash() {
    let dir = scratch();
    let primary = write_bundle(&dir, "ok.dbev", &valid_bundle());
    let earlier = write_bundle(&dir, "garbage.dbev", b"this is not a dbev container at all");
    let r = run_offline(
        bin_cli(),
        &["evidence", "verify", primary.to_str().unwrap(), "--against", earlier.to_str().unwrap()],
        &dir,
    );
    assert_eq!(r.code, 0, "primary is VALID; a garbage earlier file just skips the claim: {}", r.stderr);
    assert!(r.stdout.contains("skipped"), "{}", r.stdout);
    assert!(!r.stdout.contains("consistent"), "{}", r.stdout);
}

#[test]
fn against_a_missing_earlier_file_is_a_cli_error_3() {
    let dir = scratch();
    let primary = write_bundle(&dir, "ok.dbev", &valid_bundle());
    let r = run_offline(
        bin_cli(),
        &["evidence", "verify", primary.to_str().unwrap(), "--against", dir.join("nope.dbev").to_str().unwrap()],
        &dir,
    );
    assert_eq!(r.code, 3, "a missing --against file is a CLI error");
}

// ---- malformed input never crashes and never false-VALIDs ----

#[test]
fn verify_empty_and_garbage_files_are_cannot_verify_not_a_panic() {
    let dir = scratch();
    let empty = write_bundle(&dir, "empty.dbev", b"");
    let garbage = write_bundle(&dir, "garbage.dbev", b"\x00\x01\x02 not a zip \xff\xfe");
    for f in [empty, garbage] {
        let r = run_offline(bin_verify(), &[f.to_str().unwrap()], &dir);
        assert_eq!(r.code, 2, "malformed bytes must be CANNOT_VERIFY, never VALID/crash: {}", r.stdout);
        assert!(r.stdout.contains("CANNOT_VERIFY"), "{}", r.stdout);
    }
}

#[test]
fn verify_a_truncated_valid_bundle_never_reads_valid() {
    let dir = scratch();
    let full = valid_bundle();
    // Chop the trailing central directory / EOCD off a real bundle.
    let truncated = &full[..full.len() / 2];
    let f = write_bundle(&dir, "truncated.dbev", truncated);
    let r = run_offline(bin_verify(), &[f.to_str().unwrap()], &dir);
    assert_ne!(r.code, 0, "a truncated bundle must never read VALID: {}", r.stdout);
}

#[test]
fn against_disjoint_ranges_are_not_comparable() {
    let dir = scratch();
    // Primary: positions 1..3. Earlier: a windowed export carrying positions
    // 11..20 only. No shared positions → no claim either way.
    let primary = write_bundle(&dir, "low.dbev", &BundleBuilder::new().add_records(3).build());
    let earlier = write_bundle(
        &dir,
        "high.dbev",
        &BundleBuilder::new().add_records(20).export_window_start(10).build(),
    );
    let r = run_offline(
        bin_cli(),
        &["evidence", "verify", primary.to_str().unwrap(), "--against", earlier.to_str().unwrap()],
        &dir,
    );
    assert_eq!(r.code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stdout.contains("not-comparable"), "{}", r.stdout);
}

#[test]
fn against_skips_the_claim_when_the_earlier_bundle_is_not_valid() {
    let dir = scratch();
    // Primary VALID, earlier TAMPERED: no consistency claim, but the primary
    // verdict still stands (exit 0). Critically, NOT a false "consistent".
    let primary = write_bundle(&dir, "ok.dbev", &valid_bundle());
    let earlier = write_bundle(&dir, "bad.dbev", &tampered_bundle());
    let r = run_offline(
        bin_cli(),
        &["evidence", "verify", primary.to_str().unwrap(), "--against", earlier.to_str().unwrap()],
        &dir,
    );
    assert_eq!(r.code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stdout.contains("skipped"), "{}", r.stdout);
    assert!(!r.stdout.contains("consistent"), "must NOT claim consistency: {}", r.stdout);
}

#[test]
fn against_tampered_primary_still_exits_1() {
    let dir = scratch();
    let primary = write_bundle(&dir, "bad.dbev", &tampered_bundle());
    let earlier = write_bundle(&dir, "ok.dbev", &valid_bundle());
    let r = run_offline(
        bin_cli(),
        &["evidence", "verify", primary.to_str().unwrap(), "--against", earlier.to_str().unwrap()],
        &dir,
    );
    assert_eq!(r.code, 1, "a tampered primary is TAMPERED regardless of --against: {}", r.stderr);
}

// ═══════════════════════════════════════════════════════════════════════════
// why: refuses a non-VALID bundle; renders a record from a VALID one
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn why_refuses_a_tampered_bundle_with_exit_2() {
    let dir = scratch();
    let b = write_bundle(&dir, "bad.dbev", &tampered_bundle());
    let r = run_offline(bin_cli(), &["evidence", "why", "3", b.to_str().unwrap()], &dir);
    assert_eq!(r.code, 2, "why must refuse a non-VALID bundle: stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stderr.contains("refusing"), "{}", r.stderr);
}

#[test]
fn why_renders_a_record_from_a_valid_bundle() {
    let dir = scratch();
    let b = write_bundle(&dir, "ok.dbev", &valid_bundle());
    let r = run_offline(bin_cli(), &["evidence", "why", "3", b.to_str().unwrap()], &dir);
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("Record @position 3"), "{}", r.stdout);
}

#[test]
fn why_reports_a_missing_record_without_a_false_verdict() {
    let dir = scratch();
    let b = write_bundle(&dir, "ok.dbev", &valid_bundle());
    let r = run_offline(bin_cli(), &["evidence", "why", "999", b.to_str().unwrap()], &dir);
    assert_eq!(r.code, 0, "a not-found record in a VALID bundle is not a verdict failure");
    assert!(r.stderr.contains("No record"), "{}", r.stderr);
}

// ═══════════════════════════════════════════════════════════════════════════
// tables: populations CSV with a bundle-digest header; refuses non-VALID
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn tables_writes_a_populations_csv_with_a_digest_header() {
    let dir = scratch();
    let b = write_bundle(&dir, "ok.dbev", &valid_bundle());
    let out = dir.join("pops.csv");
    let r = run_offline(
        bin_cli(),
        &["evidence", "tables", b.to_str().unwrap(), out.to_str().unwrap()],
        &dir,
    );
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    let csv = std::fs::read_to_string(&out).expect("CSV must be written");
    assert!(csv.contains("bundle_sha256="), "digest header row missing: {csv}");
    assert!(csv.contains("verdict=VALID"), "{csv}");
    assert!(csv.contains("class,kind,count"), "{csv}");
    // All 5 builder records are class=evidence-record, kind=note.
    assert!(csv.contains("evidence-record,note,5"), "{csv}");
}

#[test]
fn tables_refuses_a_tampered_bundle_and_writes_nothing() {
    let dir = scratch();
    let b = write_bundle(&dir, "bad.dbev", &tampered_bundle());
    let out = dir.join("pops.csv");
    let r = run_offline(
        bin_cli(),
        &["evidence", "tables", b.to_str().unwrap(), out.to_str().unwrap()],
        &dir,
    );
    assert_eq!(r.code, 2, "tables must refuse a non-VALID bundle: stderr={}", r.stderr);
    assert!(!out.exists(), "no CSV may be written from an unverified bundle");
}

// ═══════════════════════════════════════════════════════════════════════════
// offline guarantee: verify/why/tables succeed with NO config and NO network
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn verify_why_tables_all_work_fully_offline_with_no_config_present() {
    // `home` is a fresh empty dir → no ~/.docbrain/config.json exists, and
    // run_offline clears DOCBRAIN_* env. If any of these three reached for a
    // server or a config, they would fail here.
    let dir = scratch();
    let home = dir.join("empty-home");
    std::fs::create_dir_all(&home).unwrap();
    assert!(!home.join(".docbrain").join("config.json").exists());

    let b = write_bundle(&dir, "ok.dbev", &valid_bundle());
    let out = dir.join("pops.csv");

    let verify = run_offline(bin_cli(), &["evidence", "verify", b.to_str().unwrap()], &home);
    assert_eq!(verify.code, 0, "offline verify: stderr={}", verify.stderr);

    let why = run_offline(bin_cli(), &["evidence", "why", "1", b.to_str().unwrap()], &home);
    assert_eq!(why.code, 0, "offline why: stderr={}", why.stderr);

    let tables = run_offline(
        bin_cli(),
        &["evidence", "tables", b.to_str().unwrap(), out.to_str().unwrap()],
        &home,
    );
    assert_eq!(tables.code, 0, "offline tables: stderr={}", tables.stderr);
    assert!(out.exists());
}
