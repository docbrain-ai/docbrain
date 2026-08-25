// SPDX-License-Identifier: MIT
//! Differential fuzzing (Task 16): THE Rust <-> Python divergence gate. The
//! frozen corpus (Task 15) proves the two verifiers agree on 23 hand-picked
//! rows; this proves it holds under seeded random and adversarial inputs. The
//! non-negotiable: on EVERY generated bundle the Rust `verify_bundle` (in
//! process) and the stdlib `tools/verify_dbev.py` (shelled to) return the
//! IDENTICAL verdict + dominant-code + dominant-row. Any divergence writes the
//! offending bundle to disk (a replayable `.dbev`) and panics with its path.
//!
//! Determinism: the generator is fully driven by a seeded SplitMix64 PRNG. The
//! seed is `EVIDENCE_FUZZ_SEED` (default a fixed constant) so a green run today
//! is green tomorrow and a red run is replayable with the printed seed. Sizes
//! are `EVIDENCE_FUZZ_N` per bucket (default 200 — the brief's CI-affordable
//! figure; the design target is 500). Each Python invocation is ~0.1-0.3s, so
//! the default ~400 invocations run in ~1-2 minutes.
//!
//! ## Buckets
//! * **valid** — random honest recipes; assert BOTH VALID and Rust==Python.
//! * **builder-mutated** — honest base + one builder mutation hook that maps to
//!   a specific taxonomy row; assert BOTH non-VALID and Rust==Python (i.e. the
//!   SAME non-VALID row — the taxonomy-parity gate).
//! * **raw-byte-mutated** — honest base + one random single-byte flip; assert
//!   Rust==Python for WHATEVER verdict results. This is where parser
//!   differentials hide (strict base64, lazy-UTF-8, hex, JSON edges).
//! * **timestamp grammar** — the controller-assigned Task-14 residual. A frozen
//!   set of unusual-but-valid RFC-3339 forms (sub-microsecond/nanosecond
//!   precision, leap-second `:60`, offset-width and case variants) injected
//!   into SIGNED checkpoint / compromise times and unsigned anchor times, plus
//!   random forms. Two of these are pinned regression vectors that reproduced
//!   the real pre-fix divergence (Python truncated to microseconds and rejected
//!   `:60`); the fix lives in Python (`parse_rfc3339`), chrono being the
//!   authoritative trust core. See tools/verify_dbev.py `_Instant`.

use docbrain_evidence::{verify_bundle, BundleBuilder, ContainerReader, ContainerWriter};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// Deterministic PRNG (SplitMix64). No `rand` dependency; fully seed-driven.
// ---------------------------------------------------------------------------
struct Rng {
    state: u64,
}
impl Rng {
    fn new(seed: u64) -> Self {
        Rng { state: seed }
    }
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `[0, n)`; `n` must be > 0.
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
    /// Uniform in `[lo, hi]` inclusive.
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }
    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
}

// ---------------------------------------------------------------------------
// Environment / paths.
// ---------------------------------------------------------------------------
const DEFAULT_SEED: u64 = 0xD0CB_2A16_1600_0016;

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <root>/crates/docbrain-evidence
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate is two levels under the workspace root")
        .to_path_buf()
}

fn python_script() -> PathBuf {
    workspace_root().join("tools/verify_dbev.py")
}

/// `true` if `python3` can be invoked at all. When absent: a hard error if
/// `EVIDENCE_FUZZ_REQUIRE` is set (CI sets it, so the gate can never silently
/// skip), otherwise a loud skip so a contributor without python3 is not blocked
/// by an unrelated crate's whole test suite.
fn python_available() -> bool {
    match Command::new("python3").arg("--version").output() {
        Ok(o) => o.status.success(),
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// The two verifiers, reduced to the parity-compared triple "VERDICT|code|row".
// ---------------------------------------------------------------------------
fn rust_verdict(bytes: &[u8]) -> String {
    let r = verify_bundle(bytes);
    format!("{}|{}|{}", r.verdict.as_str(), r.dominant.code, r.dominant.row)
}

fn python_verdict(script: &Path, tmp: &Path, bytes: &[u8]) -> Result<String, String> {
    std::fs::write(tmp, bytes).map_err(|e| format!("write temp bundle: {e}"))?;
    let out = Command::new("python3")
        .arg(script)
        .arg(tmp)
        .arg("--json")
        .output()
        .map_err(|e| format!("spawn python3: {e}"))?;
    // Exit 0/1/2 are verdicts; 3 is a CLI error (no verdict JSON). Parse stdout
    // regardless; a parse failure is itself a reportable divergence.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).map_err(|e| {
        format!(
            "python produced no parseable verdict JSON (exit {:?}): {e}; stderr={}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        )
    })?;
    let verdict = json["verdict"].as_str().ok_or("python json missing verdict")?;
    let code = json["dominant"]["code"].as_str().ok_or("python json missing dominant.code")?;
    let row = json["dominant"]["row"].as_u64().ok_or("python json missing dominant.row")?;
    Ok(format!("{verdict}|{code}|{row}"))
}

/// Writes the offending bundle to a replayable file and returns its path.
fn write_reproducer(bucket: &str, seed: u64, idx: usize, bytes: &[u8]) -> PathBuf {
    let dir = workspace_root().join("target/evidence-fuzz-repro");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("repro-{bucket}-seed{seed:016x}-{idx}.dbev"));
    if let Ok(mut f) = std::fs::File::create(&path) {
        let _ = f.write_all(bytes);
    }
    path
}

/// The core comparison. On ANY divergence, the reproducer is written FIRST,
/// then this panics with the path (never a bare `unwrap` that would lose it).
/// `expect` optionally asserts the verdict class (VALID vs non-VALID) that the
/// bucket guarantees by construction — a real false-VALID / false-TAMPERED is a
/// finding, so that assertion is not weakened away.
fn compare(
    script: &Path,
    tmp: &Path,
    bucket: &str,
    seed: u64,
    idx: usize,
    bytes: &[u8],
    expect: Option<bool>, // Some(true)=must be VALID, Some(false)=must be non-VALID
) {
    let r = rust_verdict(bytes);
    let p = match python_verdict(script, tmp, bytes) {
        Ok(p) => p,
        Err(e) => {
            let path = write_reproducer(bucket, seed, idx, bytes);
            panic!(
                "[{bucket} #{idx} seed {seed:#018x}] python side failed: {e}\n\
                 rust said [{r}]. reproducer: {}",
                path.display()
            );
        }
    };
    if r != p {
        let path = write_reproducer(bucket, seed, idx, bytes);
        panic!(
            "[{bucket} #{idx} seed {seed:#018x}] RUST<->PYTHON DIVERGENCE\n  \
             rust   = [{r}]\n  python = [{p}]\n  reproducer: {}\n  \
             replay: python3 tools/verify_dbev.py {} --json",
            path.display(),
            path.display()
        );
    }
    if let Some(want_valid) = expect {
        let is_valid = r.starts_with("VALID|");
        if is_valid != want_valid {
            let path = write_reproducer(bucket, seed, idx, bytes);
            panic!(
                "[{bucket} #{idx} seed {seed:#018x}] verdict-class violation: \
                 expected {} but both verifiers said [{r}]. reproducer: {}",
                if want_valid { "VALID" } else { "non-VALID" },
                path.display()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Generators.
// ---------------------------------------------------------------------------

/// A genuinely-honest bundle across the legal shape space (records, content,
/// honest erasure, rotation, linked anchors, windowed export). Every branch is
/// a combination the pipeline treats as VALID (informational rows 13/23/24/25
/// do not block VALID), so the caller asserts VALID.
fn gen_valid(rng: &mut Rng) -> Vec<u8> {
    let n = rng.range(0, 6);
    let mut b = BundleBuilder::new();

    // Optional rotation at a real position.
    let rotated = n >= 1 && rng.chance(1, 3);
    if rotated {
        b = b.with_rotation(rng.range(1, n));
    }
    b = b.add_records(n);

    // Content on a random subset, then honest erasure on a subset of those.
    let mut content_positions = Vec::new();
    for pos in 1..=n {
        if rng.chance(1, 2) {
            let payload = format!("content-for-{pos}-{}", rng.next_u64());
            b = b.with_content(pos, payload.as_bytes());
            content_positions.push(pos);
        }
    }
    let mut erased_any = false;
    for &pos in &content_positions {
        if rng.chance(1, 3) {
            b = b.erase(pos);
            erased_any = true;
        }
    }

    // Optional linked anchor on a real checkpoint (0 = start, n = end).
    if rng.chance(1, 2) {
        let cp = if n >= 1 && rng.chance(1, 2) { n } else { 0 };
        if rng.chance(1, 2) {
            b = b.with_anchor_witness(cp);
        } else {
            // A benign TSA time (row 23 is informational either way).
            b = b.with_anchor_token(cp, "2027-01-01T00:00:00Z");
        }
    }

    // Occasionally inject unusual-but-valid checkpoint timestamp grammar.
    if rng.chance(1, 4) {
        let (s, e) = random_valid_ts_pair(rng);
        b = b.with_checkpoint_times(&s, &e);
    }

    // Optional windowed export. Kept simple: only when nothing was erased (so
    // no erasure/window accounting interaction), with a mid-range start the
    // builder hashes the excluded records into.
    if n >= 2 && !erased_any && rng.chance(1, 4) {
        b = b.export_window_start(rng.range(1, n - 1));
    }

    b.build()
}

/// A simple honest base for the raw-byte bucket (small, fast, varied).
fn gen_valid_simple(rng: &mut Rng) -> Vec<u8> {
    let n = rng.range(1, 4);
    let mut b = BundleBuilder::new().add_records(n);
    if rng.chance(1, 2) {
        b = b.with_content(1, b"raw-byte-base-content");
    }
    b.build()
}

/// An honest base plus exactly ONE builder mutation hook that maps to a
/// specific blocking taxonomy row — guaranteed non-VALID. Returns the bundle;
/// the caller asserts non-VALID and Rust==Python (i.e. the SAME row).
fn gen_builder_mutated(rng: &mut Rng) -> Vec<u8> {
    // Each arm builds its own base with the prerequisites its mutation needs.
    match rng.below(14) {
        0 => {
            let pos = rng.range(1, 4);
            BundleBuilder::new().add_records(4).tamper_record(pos).build() // row 2
        }
        1 => {
            let pos = rng.range(1, 4);
            BundleBuilder::new().add_records(4).forge_prev_head(pos).build() // row 4
        }
        2 => {
            let pos = rng.range(1, 4);
            BundleBuilder::new().add_records(4).unknown_key_record(pos).build() // row 16
        }
        3 => {
            let pos = rng.range(1, 4);
            BundleBuilder::new().add_records(4).multi_sig_record(pos).build() // row 17
        }
        4 => {
            let pos = rng.range(1, 4);
            BundleBuilder::new().add_records(4).wrong_payload_type_record(pos).build() // row 22
        }
        5 => BundleBuilder::new().add_records(4).tamper_manifest().build(), // row 11
        6 => BundleBuilder::new()
            .add_records(4)
            .with_content(2, b"tamper-me")
            .tamper_content(2)
            .build(), // row 12
        7 => BundleBuilder::new().add_records(4).mismatched_manifest_counts().build(), // row 10
        8 => BundleBuilder::new()
            .add_records(4)
            .mismatched_manifest_export_checkpoint()
            .build(), // row 10
        9 => BundleBuilder::new()
            .add_records(4)
            .with_content(3, b"gone")
            .drop_erasure(3)
            .build(), // row 14
        10 => BundleBuilder::new().add_records(4).duplicate_member("checkpoints.jsonl").build(), // row 21
        11 => BundleBuilder::new()
            .add_records(4)
            .missing_container_member("trust/keys.jsonl")
            .build(), // row 21
        12 => {
            // row 3: forge across a rotation so the "wrong-era" key genuinely
            // differs from the one authoritative at the record's position.
            BundleBuilder::new().with_rotation(3).add_records(4).forge_position(4, 1).build()
        }
        _ => BundleBuilder::new().add_records(4).dangling_erasure_target(9999).build(), // row 22
    }
}

// ---------------------------------------------------------------------------
// Timestamp grammar (the controller-assigned Task-14 residual).
// ---------------------------------------------------------------------------

/// A pair of unusual-but-VALID RFC-3339 forms for the start/end checkpoints.
/// Every element parses in BOTH chrono and the fixed Python parser; the pair is
/// used as SIGNED checkpoint times, so it reaches the verifier's parse/compare
/// stage exactly as an honest export would.
fn random_valid_ts_pair(rng: &mut Rng) -> (String, String) {
    // Forms proven (Task-16 probe) to parse identically in chrono and Python.
    let forms: &[&str] = &[
        "2026-01-01T00:00:00Z",
        "2026-01-01T00:00:00.000000500Z", // sub-microsecond
        "2026-01-01T00:00:00.123456789Z", // nanosecond
        "2026-01-01T00:00:60Z",           // leap second
        "2026-01-01T00:01:00Z",           // the second immediately after :00:60
        "2026-06-30T23:59:60Z",           // leap second at the day boundary
        "2026-07-01T00:00:00Z",           // the whole second the leap precedes
        "2026-01-01T00:00:00+00:00",      // offset width
        "2026-01-01T00:00:00+23:59",      // extreme valid offset
        "2026-01-01t00:00:00z",           // lowercase t/z
        "2026-01-01T00:00:00.5Z",
        "2026-01-01T01:00:00Z",
    ];
    let s = forms[rng.below(forms.len() as u64) as usize].to_string();
    let e = forms[rng.below(forms.len() as u64) as usize].to_string();
    (s, e)
}

/// The FROZEN timestamp regression vectors. `(builder, label, expect_valid)`.
/// Two of them (`subus-no-anomaly`, `leap-second-parse`) reproduced the real
/// pre-fix Rust<->Python divergence; all must now agree.
fn timestamp_vectors() -> Vec<(Vec<u8>, &'static str, Option<bool>)> {
    let mut v: Vec<(Vec<u8>, &'static str, Option<bool>)> = Vec::new();

    // Regression vector 1: sub-microsecond, end AFTER start → no clock anomaly.
    // Pre-fix Python truncated both to 0us and reported a spurious row-24
    // anomaly (dominant clock-anomaly) while Rust saw none (dominant valid).
    v.push((
        BundleBuilder::new()
            .add_records(2)
            .with_checkpoint_times("2026-01-01T00:00:00.000000000Z", "2026-01-01T00:00:00.000000500Z")
            .build(),
        "subus-no-anomaly",
        Some(true),
    ));
    // Regression vector 2: leap-second in a SIGNED checkpoint time. Pre-fix
    // Python raised on `:60` (row 22 CANNOT_VERIFY) while Rust parsed it VALID.
    v.push((
        BundleBuilder::new()
            .add_records(2)
            .with_checkpoint_times("2026-01-01T00:00:00Z", "2026-01-01T00:00:60Z")
            .build(),
        "leap-second-parse",
        Some(true),
    ));
    // Sub-microsecond where end is BEFORE start by 500ns → a genuine clock
    // anomaly (row 24, informational) that BOTH must now report (still VALID).
    v.push((
        BundleBuilder::new()
            .add_records(2)
            .with_checkpoint_times("2026-01-01T00:00:00.000000500Z", "2026-01-01T00:00:00.000000000Z")
            .build(),
        "subus-real-anomaly",
        Some(true),
    ));
    // Nanosecond precision, both parse, end after start.
    v.push((
        BundleBuilder::new()
            .add_records(2)
            .with_checkpoint_times("2026-01-01T00:00:00.123456789Z", "2026-01-01T00:00:00.123456790Z")
            .build(),
        "nanosecond",
        Some(true),
    ));
    // Offset-width / case variants — must parse identically.
    v.push((
        BundleBuilder::new()
            .add_records(2)
            .with_checkpoint_times("2026-01-01T00:00:00+00:00", "2026-01-01t01:00:00z")
            .build(),
        "offset-and-case",
        Some(true),
    ));
    // Parse-FAILURE parity: a second-61 in a signed checkpoint time — BOTH must
    // reject it (row 22, CANNOT_VERIFY), never one accept.
    v.push((
        BundleBuilder::new()
            .add_records(2)
            .with_checkpoint_times("2026-01-01T00:00:00Z", "2026-01-01T00:00:61Z")
            .build(),
        "second-61-both-reject",
        Some(false),
    ));
    // Invalid month in a signed checkpoint time — BOTH reject.
    v.push((
        BundleBuilder::new()
            .add_records(2)
            .with_checkpoint_times("2026-01-01T00:00:00Z", "2026-13-01T00:00:00Z")
            .build(),
        "month-13-both-reject",
        Some(false),
    ));
    // Leap second in a SIGNED compromise time (reaches the key-chain parse).
    // Pre-fix Python raised (row 22) while Rust classified the compromised-key
    // records as row-7 TAMPERED; now both agree.
    v.push((
        BundleBuilder::new()
            .with_recovery_key()
            .with_rotation(1)
            .with_compromise(2, "2026-06-30T23:59:60Z")
            .add_records(3)
            .build(),
        "compromise-leap-second",
        None, // non-VALID (row 7), but assert only Rust==Python here
    ));
    // Sub-microsecond in an unsigned anchor TSA time vs a whole-second
    // checkpoint (row 23 comparison surface).
    v.push((
        BundleBuilder::new()
            .add_records(2)
            .with_anchor_token(2, "2026-01-01T00:00:59.999999500Z")
            .build(),
        "anchor-tsa-subus",
        Some(true),
    ));

    // Fix-round-1 regression vector (I1): a `:60` leap second is STRICTLY
    // before the following whole second in chrono (probe: lt=true, eq=false),
    // so end (`00:01:00`) is after start (`00:00:60`) → no clock anomaly →
    // VALID|valid. The pre-fix flat-1e9 key collapsed `:60` onto `00:01:00`
    // (equal), so Python reported a spurious row-24 clock-anomaly while Rust
    // said valid — a triple divergence this pins forever.
    v.push((
        BundleBuilder::new()
            .add_records(2)
            .with_checkpoint_times("2026-01-01T00:00:60Z", "2026-01-01T00:01:00Z")
            .build(),
        "leap-adjacent-next-second",
        Some(true),
    ));
    // Fix-round-1 regression vector (I1), the reviewer's exact day-boundary
    // case: `23:59:60` strictly precedes next-day midnight.
    v.push((
        BundleBuilder::new()
            .add_records(2)
            .with_checkpoint_times("2026-06-30T23:59:60Z", "2026-07-01T00:00:00Z")
            .build(),
        "leap-boundary-next-day",
        Some(true),
    ));
    // Fix-round-1 regression vector (C1): an out-of-range offset MINUTE in a
    // SIGNED checkpoint time. chrono rejects `+00:60` ("out of range") → row 22
    // CANNOT_VERIFY; both verifiers must agree. The pre-fix Python silently
    // read `+00:60` as +1h and returned VALID — a false-VALID in the published
    // reference verifier, the worst-direction violation.
    v.push((
        BundleBuilder::new()
            .add_records(2)
            .with_checkpoint_times("2026-01-01T00:00:00Z", "2026-01-01T00:00:00+00:60")
            .build(),
        "offset-minute-60-both-reject",
        Some(false),
    ));
    // Fix-round-1 regression vector (C1): out-of-range offset HOUR.
    v.push((
        BundleBuilder::new()
            .add_records(2)
            .with_checkpoint_times("2026-01-01T00:00:00Z", "2026-01-01T00:00:00+24:00")
            .build(),
        "offset-hour-24-both-reject",
        Some(false),
    ));
    v
}

// ---------------------------------------------------------------------------
// Frozen duplicate-JSON-key regression vectors. A duplicate of a KNOWN struct
// field is the one dup-key case that reaches a parse in a full bundle without
// FIRST tripping a member-hash or signature gate: it can only live in the
// manifest's outer DSSE envelope, whose bytes the manifest does NOT self-hash
// (it self-authenticates via its own signature, and that signature covers only
// pae(payloadType, payload) — never the envelope's own keyid/sig). serde's
// derived Deserialize rejects the duplicate ("duplicate field `keyid`" -> row
// 22); the Python verifier's json.loads used to SILENTLY keep the last value
// and return VALID -> a false-VALID in the public "trust-nobody" auditor. The
// fix is a json.loads `object_pairs_hook` in verify_dbev.py that rejects ANY
// duplicate key at the single chokepoint every derived-struct parse routes
// through. The raw-byte-mutated bucket CANNOT reach this (a single-byte flip
// cannot INSERT a key), which is why these are frozen, hand-built vectors.
// ---------------------------------------------------------------------------

/// Rebuilds `bundle` with one member's bytes replaced by `mutate(member)`,
/// preserving central-directory order and storing every member verbatim.
fn repack_member(bundle: &[u8], target: &str, mutate: impl Fn(&[u8]) -> Vec<u8>) -> Vec<u8> {
    let reader = ContainerReader::open(bundle).expect("honest bundle opens");
    let names: Vec<String> = reader.member_names().to_vec();
    let mut w = ContainerWriter::new();
    for name in &names {
        let bytes = reader.member_bytes(name).expect("member present").to_vec();
        let data = if name == target { mutate(&bytes) } else { bytes };
        w.add_member(name, data).expect("re-add member");
    }
    w.finish().expect("repack")
}

/// Duplicates `key` in a single-object member (e.g. manifest.json) by inserting
/// `,"key":<value>` immediately before the object's final `}`. `key` MUST
/// already be present so this is a genuine duplicate, not a new field.
fn dup_key_in_object(member: &[u8], key: &str, value: &str) -> Vec<u8> {
    let s = std::str::from_utf8(member).expect("utf8 member");
    let cut = s.rfind('}').expect("closing brace");
    format!("{}{}{}", &s[..cut], format!(",\"{key}\":{value}"), &s[cut..]).into_bytes()
}

/// Duplicates `key` on the FIRST JSONL line's outer object (e.g. the first
/// checkpoint envelope), before that line's final `}`.
fn dup_key_in_first_line(member: &[u8], key: &str, value: &str) -> Vec<u8> {
    let s = std::str::from_utf8(member).expect("utf8 member");
    let nl = s.find('\n').unwrap_or(s.len());
    let (line, rest) = s.split_at(nl);
    let cut = line.rfind('}').expect("closing brace on first line");
    format!("{}{}{}{}", &line[..cut], format!(",\"{key}\":{value}"), &line[cut..], rest).into_bytes()
}

/// The FROZEN dup-key regression vectors. `(bundle, label, expect_valid)`.
fn dupkey_vectors() -> Vec<(Vec<u8>, &'static str, Option<bool>)> {
    let base = BundleBuilder::new().add_records(3).build();
    let mut v: Vec<(Vec<u8>, &'static str, Option<bool>)> = Vec::new();

    // LOAD-BEARING vector: duplicate the KNOWN `keyid` field in manifest.json's
    // outer envelope. serde -> "duplicate field `keyid`" (row 22); pre-fix
    // Python kept the last value and returned VALID. This is the exact
    // divergence the fix closes: reverting the `object_pairs_hook` makes Python
    // flip back to VALID|valid|1 and this vector panics on the mismatch.
    v.push((
        repack_member(&base, "manifest.json", |m| dup_key_in_object(m, "keyid", "\"deadbeef\"")),
        "manifest-outer-dup-keyid",
        Some(false), // both CANNOT_VERIFY | cannot-verify-malformed | row 22
    ));

    // Coverage vector on ANOTHER member: a duplicate key inside the first
    // checkpoint envelope. checkpoints.jsonl IS SHA-256-locked by the manifest,
    // so BOTH verifiers catch the mutated bytes as a member-hash mismatch (row
    // 12, TAMPERED) BEFORE json_parse is reached — proving the two verifiers
    // still agree on the SAME dominant row wherever a dup key lands, and that a
    // dup key in a hash-locked member is unreachable to json_parse (defense in
    // depth). This vector agrees before AND after the fix; it guards hash-gate
    // parity, not the json_parse chokepoint (the manifest vector guards that).
    v.push((
        repack_member(&base, "checkpoints.jsonl", |m| dup_key_in_first_line(m, "keyid", "\"deadbeef\"")),
        "checkpoints-member-dup-keyid",
        Some(false), // both TAMPERED | tampered-content | row 12
    ));

    // LOAD-BEARING vector: a duplicate of an UNKNOWN key inside the anchor
    // member. The anchor is the ONE bundle parse with no `deny_unknown_fields`
    // closed-schema struct to backstop it: serde's `AnchorPeek` silently keeps
    // the last value and reads VALID, while Python's `object_pairs_hook`
    // rejects the duplicate as `anchor-invalid`. The Rust anchor parse now runs
    // a recursive `NoDupKeys` pre-scan so BOTH land on VALID | anchor-invalid |
    // row 18 (informational — the bundle is otherwise VALID). Reverting that
    // pre-scan flips Rust back to VALID | valid | row 1 and this vector panics
    // on the dominant-code/row mismatch. `with_anchor_witness_raw_suffix` splices
    // the raw `"zzz":1,"zzz":2` before the anchor object's closing brace at
    // BUILD time, so the manifest hashes the dup-bearing bytes (no row-12
    // member-hash mismatch — the dup genuinely reaches the parse).
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_anchor_witness_raw_suffix(0, "\"zzz\":1,\"zzz\":2")
            .build(),
        "anchor-dup-unknown-key",
        Some(true), // both VALID | anchor-invalid | row 18 (informational)
    ));

    // Forward-compat / no-over-rejection vector: a SINGLE unknown key in the
    // anchor. The dup-rejection must NOT reject a lone unknown key — a v1.1
    // bundle carrying a new anchor field must still verify VALID on a v1
    // verifier, and Python tolerates a single unknown key too (its hook rejects
    // only DUPLICATES). Both must read VALID | valid | row 1, tier granted.
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_anchor_witness_raw_suffix(0, "\"zzz\":1")
            .build(),
        "anchor-single-unknown-key",
        Some(true), // both VALID | valid | row 1
    ));

    // LOAD-BEARING vector: INVALID UTF-8 (a raw 0xFF byte) inside an IGNORED
    // anchor field's VALUE. The anchor parse is deliberately tolerant (no
    // deny_unknown), so `junk` is a field NEITHER verifier reads — but the
    // byte's VALIDITY still matters. Rust's NoDupKeys pre-scan (serde_json
    // deserialize_any) visits every string value, ignored fields included, and
    // rejects the bad byte -> VALID | anchor-invalid | row 18. Pre-fix, Python's
    // lazy surrogateescape parse let the bad byte survive inside the ignored
    // value and read the anchor VALID | valid | row 1 | tier witness-file-present
    // — a divergence on dominant code AND row AND tier. The fix makes Python
    // require the anchor member to be WHOLLY valid UTF-8 (row 18 on a bad byte),
    // rejecting only invalid UTF-8, never unknown fields. Reverting that Python
    // UTF-8 check flips Python back to VALID | valid | row 1 and this vector
    // PANICS on the mismatch — the revert-and-fail proof for the fix. The 0xFF
    // is why `with_anchor_witness_raw_bytes_suffix` (raw bytes, not the &str
    // suffix) is required: a Rust &str cannot carry a 0xFF byte.
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_anchor_witness_raw_bytes_suffix(0, b"\"junk\":\"\xff\"")
            .build(),
        "anchor-badutf8-ignored-value",
        Some(true), // both VALID | anchor-invalid | row 18 (informational)
    ));

    // Hostile-input hardening vector: an anchor whose IGNORED `junk` field is
    // JSON nested far past any recursion limit. A verifier must map hostile
    // input to a VERDICT, never crash. Rust's NoDupKeys pre-scan rejects at
    // serde_json's recursion cap; Python's json.loads raises RecursionError,
    // which the fix maps to the anchor-invalid path (not an uncaught traceback /
    // exit 3). At this depth BOTH land on VALID | anchor-invalid | row 18. Pre-
    // fix, Python raised an uncaught RecursionError — no verdict JSON — and the
    // gate's `python_verdict` would report "python side failed" (a crash, not a
    // divergence). Depth 20000 is chosen ABOVE json.loads' own C-scanner limit so
    // Python rejects at parse (row 18), keeping both sides on the SAME row rather
    // than in the shallower window where the two recursion caps disagree.
    let deep_junk = {
        let n = 20_000usize;
        let mut s: Vec<u8> = b"\"junk\":".to_vec();
        s.extend(std::iter::repeat(b'[').take(n));
        s.extend(std::iter::repeat(b']').take(n));
        s
    };
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_anchor_witness_raw_bytes_suffix(0, &deep_junk)
            .build(),
        "anchor-deep-nesting-no-crash",
        Some(true), // both VALID | anchor-invalid | row 18 (a VERDICT, no crash)
    ));
    v
}

// ---------------------------------------------------------------------------
// Frozen serde_json-strictness regression vectors. `json.loads` is more LENIENT
// than `serde_json` in two STRUCTURAL ways the parser enforces for every byte it
// tokenizes (read AND ignored fields), so the fix lives in the Python verifier's
// single `json_parse` chokepoint (tools/verify_dbev.py), Rust being the
// authoritative target:
//   * non-finite constants — `NaN`/`Infinity`/`-Infinity` are valid in Python's
//     json but rejected by serde_json (measured: an anchor whose IGNORED field is
//     `NaN` is VALID | anchor-invalid | row 18 in Rust, but was VALID | valid |
//     row 1 in Python — a false-VALID). Fix: a `parse_constant` that raises.
//   * recursion depth — serde_json caps container nesting at 128 (measured
//     directly against serde_json: it ACCEPTS a value nested 127 deep and REJECTS
//     the 128th open `[`/`{`, identically for `Value` and the anchor `NoDupKeys`
//     pre-scan); json.loads allows ~1000. Fix: a pre-scan raising at the IDENTICAL
//     128-open-bracket boundary. Both edges are pinned below (127 accept, 128
//     reject) because an off-by-one at the boundary is itself a divergence.
//
// The anchor is the ONLY bundle parse with no `deny_unknown_fields`/typed-Value
// backstop, so an anchor whose IGNORED field carries the malformed token is the
// discriminating vehicle: it flips VALID|valid|row1|tier ↔ VALID|anchor-invalid|
// row18 on the Python strictness alone (reverting the fix flips Python back and
// PANICS the gate — the load-bearing proof). `with_anchor_witness_raw_bytes_suffix`
// splices the raw bytes before the anchor object's closing brace at BUILD time,
// so the manifest hashes them (the token genuinely reaches the parse, no row-12
// member-hash gate first). The manifest strict-member vectors freeze that the
// SAME tokens in a `deny_unknown` member land on CANNOT_VERIFY|malformed|row 22
// on both sides (there a type-check/multi-sig gate also catches them, so those
// are coverage, not load-bearing).
// ---------------------------------------------------------------------------

/// Inject `,"<field>":<value_raw>` before the manifest envelope object's final
/// `}` (the manifest's OUTER DSSE envelope is NOT self-hashed, so the mutation
/// reaches the parse un-gated — same un-hash-gated slot the dup-keyid vector uses).
fn manifest_env_add_field(bundle: &[u8], field: &str, value_raw: &str) -> Vec<u8> {
    repack_member(bundle, "manifest.json", |m| {
        let s = std::str::from_utf8(m).expect("utf8 manifest env");
        let cut = s.rfind('}').expect("closing brace");
        format!("{}{}{}{}", &s[..cut], format!(",\"{field}\":"), value_raw, &s[cut..]).into_bytes()
    })
}

/// Corrupt the first byte of the manifest envelope's `keyid` STRING value to a
/// raw `0xFF` — invalid UTF-8 inside a field `WireEnvelope` READS (deny_unknown).
fn manifest_env_badutf8_keyid(bundle: &[u8]) -> Vec<u8> {
    repack_member(bundle, "manifest.json", |m| {
        let needle: &[u8] = b"\"keyid\":\"";
        let pos = m.windows(needle.len()).position(|w| w == needle).expect("keyid field");
        let mut out = m.to_vec();
        out[pos + needle.len()] = 0xFF;
        out
    })
}

/// The FROZEN serde_json-strictness regression vectors. `(bundle, label, expect_valid)`.
fn strictness_vectors() -> Vec<(Vec<u8>, &'static str, Option<bool>)> {
    let mut v: Vec<(Vec<u8>, &'static str, Option<bool>)> = Vec::new();

    // LOAD-BEARING (parse_constant): `NaN` in an IGNORED anchor field. Rust's
    // NoDupKeys deserialize_any rejects the token → VALID | anchor-invalid | 18.
    // Reverting the Python `parse_constant` flips Python to VALID | valid | 1 and
    // PANICS this vector on the dominant-code/row/tier mismatch.
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_anchor_witness_raw_bytes_suffix(0, b"\"junk\":NaN")
            .build(),
        "anchor-nan-ignored-value",
        Some(true), // both VALID | anchor-invalid | row 18
    ));
    // Coverage: `Infinity` (the other non-finite constant) in the same slot.
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_anchor_witness_raw_bytes_suffix(0, b"\"junk\":Infinity")
            .build(),
        "anchor-infinity-ignored-value",
        Some(true), // both VALID | anchor-invalid | row 18
    ));

    // LOAD-BEARING (depth pre-scan): 127 nested arrays in the IGNORED anchor
    // field — 128 simultaneously-open brackets counting the anchor object — is
    // serde_json's FIRST over-limit depth → VALID | anchor-invalid | 18.
    // Reverting the Python depth check flips Python to VALID | valid | 1 and
    // PANICS this vector.
    let nest_128_open = {
        let mut s: Vec<u8> = b"\"junk\":".to_vec();
        s.extend(std::iter::repeat(b'[').take(127));
        s.extend(std::iter::repeat(b']').take(127));
        s
    };
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_anchor_witness_raw_bytes_suffix(0, &nest_128_open)
            .build(),
        "anchor-nest-128-open-reject",
        Some(true), // both VALID | anchor-invalid | row 18
    ));
    // OFF-BY-ONE GUARD: 126 nested arrays — 127 open brackets — is serde_json's
    // LAST accepted depth → VALID | valid | 1, tier granted. If the Python depth
    // check rejected at 127 (one too eager) this vector would flip Python to
    // row 18 and PANIC — so it pins the ACCEPT edge exactly where Rust's is.
    let nest_127_open = {
        let mut s: Vec<u8> = b"\"junk\":".to_vec();
        s.extend(std::iter::repeat(b'[').take(126));
        s.extend(std::iter::repeat(b']').take(126));
        s
    };
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_anchor_witness_raw_bytes_suffix(0, &nest_127_open)
            .build(),
        "anchor-nest-127-open-accept",
        Some(true), // both VALID | valid | row 1 (tier witness-file-present)
    ));

    // ----- out-of-f64-range NUMBERS (the fourth serde_json-strictness axis) -----
    // `serde_json::Value` / the anchor `NoDupKeys` pre-scan EAGERLY materialize
    // every number (ignored fields included), so a value outside f64 range errors
    // "number out of range" -> VALID | anchor-invalid | row 18. json.loads is
    // lenient (`1e400` -> `inf` WITHOUT firing `parse_constant`, which only catches
    // the NaN/Infinity barewords; a 345-digit int -> a Python bigint), so pre-fix
    // these read VALID | valid | row 1 — a false disagreement. The fix
    // (tools/verify_dbev.py `_reject_anchor_out_of_range_numbers`, ANCHOR-LOCAL —
    // NOT in `json_parse`, because the LAZY record/checkpoint parses skip an
    // ignored number without reconstructing an f64, so Rust accepts a giant number
    // there and a global reject would over-reject) makes both land on row 18.

    // LOAD-BEARING: `1e400` (overflows f64) in an IGNORED anchor field. Reverting
    // the Python number hooks flips Python to VALID | valid | 1 and PANICS this
    // vector on the dominant-code/row/tier mismatch.
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_anchor_witness_raw_bytes_suffix(0, b"\"junk\":1e400")
            .build(),
        "anchor-number-overflow-e400",
        Some(true), // both VALID | anchor-invalid | row 18
    ));
    // LOAD-BEARING: a 345-digit integer (serde reconstructs significand*10^exp ->
    // infinite -> rejects) in an IGNORED anchor field.
    let giant_int = {
        let mut s: Vec<u8> = b"\"junk\":1".to_vec();
        s.extend(std::iter::repeat(b'0').take(344));
        s
    };
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_anchor_witness_raw_bytes_suffix(0, &giant_int)
            .build(),
        "anchor-giant-int-overflow",
        Some(true), // both VALID | anchor-invalid | row 18
    ));
    // LOAD-BEARING (float ULP boundary): `1.797693134862315708e308` — serde's
    // significand*10^exp reconstruction OVERFLOWS (rejects), but Python's
    // correct-rounding `float()` keeps it finite (== f64::MAX). A naive
    // `float()`-finite hook would ACCEPT here and flip Python to row 1 -> PANIC;
    // the exact serde-reconstruction replica rejects, matching Rust. This pins the
    // razor-thin band just under f64::MAX where correct rounding and serde differ.
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_anchor_witness_raw_bytes_suffix(0, b"\"junk\":1.797693134862315708e308")
            .build(),
        "anchor-number-f64max-ulp-reject",
        Some(true), // both VALID | anchor-invalid | row 18
    ));
    // OVER-REJECTION GUARD: `1e308` is IN f64 range -> serde ACCEPTS -> the anchor
    // is otherwise honest -> VALID | valid | row 1, tier granted. If the Python
    // number hook rejected an in-range value (too eager) this would flip Python to
    // row 18 and PANIC — so it pins the ACCEPT edge exactly where Rust's is.
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_anchor_witness_raw_bytes_suffix(0, b"\"junk\":1e308")
            .build(),
        "anchor-number-1e308-inrange",
        Some(true), // both VALID | valid | row 1 (tier witness-file-present)
    ));

    // Coverage (strict member): `NaN` as the value of the KNOWN `signatures`
    // field of the manifest's outer envelope. serde_json rejects the token at
    // parse; the Python side reaches the same CANNOT_VERIFY | malformed | 22
    // (both parse-reject after the fix, and a multi-signature/type gate before
    // it, so this freezes strict-member parity without being load-bearing).
    let strict_base = BundleBuilder::new().add_records(3).build();
    v.push((
        manifest_env_add_field(&strict_base, "signatures", "NaN"),
        "manifest-strict-member-nan",
        Some(false), // both CANNOT_VERIFY | cannot-verify-malformed | row 22
    ));
    // Coverage (strict member): 200 nested arrays in the KNOWN `signatures`
    // field — over serde's depth cap in a member WireEnvelope reads as a Value.
    v.push((
        manifest_env_add_field(
            &strict_base,
            "signatures",
            &format!("{}{}", "[".repeat(200), "]".repeat(200)),
        ),
        "manifest-strict-member-deep-nesting",
        Some(false), // both CANNOT_VERIFY | cannot-verify-malformed | row 22
    ));
    // Coverage (strict member): invalid UTF-8 (0xFF) inside the manifest
    // envelope's `keyid` string — a field WireEnvelope READS. Python's per-field
    // UTF-8 apparatus (`_ck_str`, unchanged by this pass) already rejects it; this
    // freezes that a bad byte in a READ strict-member field stays row 22 on both.
    v.push((
        manifest_env_badutf8_keyid(&strict_base),
        "manifest-strict-member-badutf8-read-field",
        Some(false), // both CANNOT_VERIFY | cannot-verify-malformed | row 22
    ));

    v
}

// ---------------------------------------------------------------------------
// Frozen UNIFORM-strict-profile regression vectors: the SAME six-axis strict
// profile now applies to EVERY JSON member on both sides (not only anchors), so
// a malformed RECORD BODY / ENVELOPE field lands on the identical row. These
// close the record/envelope divergences the anchor-only fix left open:
//
//   * a duplicate key INSIDE a record `body` (a `serde_json::Value` field): the
//     Rust `Value` map decode KEEPS-LAST and silently accepted it (VALID) while
//     Python's global `object_pairs_hook` rejected it (row 22). The Rust
//     `from_slice_strict` pre-pass on the record payload now rejects it too.
//   * an out-of-f64-range number in a record body / an ignored body field: the
//     Rust `Value` decode EAGERLY rejected it (row 22) while Python's json.loads
//     turned `1e400`→inf / a giant int→bigint and accepted (VALID). Python's
//     now-GLOBAL number-range hooks (in `json_parse`) reject it too.
//
// The record body is SIGNED and member-HASHED over the malformed bytes at build
// time (`with_record_raw_body`, which `serde_json::json!` cannot emit), so the
// malformation genuinely reaches the record PARSE rather than tripping a
// member-hash mismatch (row 12) first — the discriminating vehicle, exactly like
// the anchor raw-suffix vectors but for a hash-locked strict member.
// ---------------------------------------------------------------------------

/// The FROZEN uniform-strict-profile record/envelope vectors. `(bundle, label, expect_valid)`.
fn uniform_strict_vectors() -> Vec<(Vec<u8>, &'static str, Option<bool>)> {
    let mut v: Vec<(Vec<u8>, &'static str, Option<bool>)> = Vec::new();

    // LOAD-BEARING (Rust pre-pass): a duplicate key inside a record `body`
    // Value. Pre-fix Rust kept-last and read VALID; Python rejected (row 22).
    // Reverting the Rust `from_slice_strict` pre-pass on the record payload flips
    // Rust back to VALID|valid|1 and PANICS this vector on the mismatch.
    v.push((
        BundleBuilder::new().add_records(3).with_record_raw_body(2, b"{\"a\":1,\"a\":2}").build(),
        "record-body-dup-key",
        Some(false), // both CANNOT_VERIFY | cannot-verify-malformed | row 22
    ));

    // LOAD-BEARING (Python number-range): an out-of-f64-range number in an
    // IGNORED record body field. Pre-fix Python accepted (json.loads `1e400`→inf)
    // and read VALID; Rust rejected (Value eager, row 22). Reverting the Python
    // `_strict_parse_int`/`_strict_parse_float` hooks flips Python back to
    // VALID|valid|1 and PANICS this vector on the mismatch.
    v.push((
        BundleBuilder::new().add_records(3).with_record_raw_body(2, b"{\"junk\":1e400}").build(),
        "record-ignored-field-out-of-range-number",
        Some(false), // both CANNOT_VERIFY | cannot-verify-malformed | row 22
    ));
    // Coverage: the giant-integer form of the same axis (serde reconstructs
    // significand*10^exp → non-finite → rejects; json.loads → bigint pre-fix).
    let giant = {
        let mut s: Vec<u8> = b"{\"junk\":1".to_vec();
        s.extend(std::iter::repeat(b'0').take(344));
        s.push(b'}');
        s
    };
    v.push((
        BundleBuilder::new().add_records(3).with_record_raw_body(2, &giant).build(),
        "record-body-giant-int-out-of-range",
        Some(false), // both CANNOT_VERIFY | cannot-verify-malformed | row 22
    ));

    // Coverage (envelope-signatures out-of-range number): `1e400` as the value of
    // the KNOWN `signatures` field of the manifest's OUTER (un-hash-gated) DSSE
    // envelope. Rust rejects at the `WireEnvelope` Value decode; Python now
    // rejects at `json_parse` (pre-fix it reached the multi-sig → Unsupported →
    // Malformed path, coincidentally also row 22). Both land on row 22 either way,
    // so this is coverage, not load-bearing.
    let strict_base = BundleBuilder::new().add_records(3).build();
    v.push((
        manifest_env_add_field(&strict_base, "signatures", "1e400"),
        "envelope-signatures-out-of-range-number",
        Some(false), // both CANNOT_VERIFY | cannot-verify-malformed | row 22
    ));

    // Coverage (deep-nest strict member): a record body nested 200 deep — over
    // serde_json's 128 recursion cap in a HASH-LOCKED strict member (distinct
    // from the existing manifest-envelope deep-nest vector). Rust rejects at the
    // Value recursion cap; Python at `_check_json_depth`. Both row 22.
    let deep = {
        let mut s: Vec<u8> = Vec::new();
        s.extend(std::iter::repeat(b'[').take(200));
        s.extend(std::iter::repeat(b']').take(200));
        s
    };
    v.push((
        BundleBuilder::new().add_records(3).with_record_raw_body(2, &deep).build(),
        "deep-nest-strict-member",
        Some(false), // both CANNOT_VERIFY | cannot-verify-malformed | row 22
    ));

    // OVER-REJECTION GUARDS: honest, in-range record bodies must stay VALID on
    // both — the strict pre-pass must not reject legitimate content. If the
    // number-range hook were too eager (rejecting an in-range value), or the
    // pre-pass rejected a legal nested object, these would flip to row 22 and
    // PANIC — pinning the ACCEPT edge exactly where Rust's is.
    v.push((
        BundleBuilder::new().add_records(3).with_record_raw_body(2, b"{\"n\":1e308}").build(),
        "record-body-inrange-e308-accept",
        Some(true), // both VALID | valid | row 1
    ));
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_record_raw_body(2, b"{\"a\":{\"b\":[1,2,3]},\"s\":\"caf\\u00e9\"}")
            .build(),
        "record-body-nested-unicode-accept",
        Some(true), // both VALID | valid | row 1 (multibyte UTF-8 via \u escape)
    ));
    // A 26-digit integer is > u64 but reconstructs to a FINITE f64 (~1.23e25) —
    // serde ACCEPTS it, so both must (pins the number ACCEPT edge above u64).
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_record_raw_body(2, b"{\"n\":12345678901234567890123456}")
            .build(),
        "record-body-above-u64-inrange-accept",
        Some(true), // both VALID | valid | row 1
    ));

    v
}

// ---------------------------------------------------------------------------
// Frozen LONE-SURROGATE regression vectors: the SEVENTH axis of the uniform
// strict-JSON profile. serde_json's strings are Rust `String`s (valid Unicode
// SCALAR values only), so its eager `from_slice_strict` pre-pass REJECTS a
// `\uXXXX` escape that forms a lone/unpaired UTF-16 surrogate (U+D800..U+DFFF)
// — "unexpected end of hex escape". Python's json.loads is LENIENT: it ACCEPTS
// `\uD800` and returns a `str` holding the lone surrogate code point. A decode
// of the input BYTES is NOT sufficient to catch this — the bytes `\uD800` are
// plain ASCII and decode fine; the gap is that the DECODED string is not a valid
// scalar sequence. The fix (tools/verify_dbev.py `_reject_lone_surrogates`, run
// at the `json_parse` chokepoint after json.loads) walks every decoded string
// (keys AND values, every level) and rejects a lone surrogate, matching serde
// exactly. A valid surrogate PAIR (emoji) is COMBINED by json.loads into one
// scalar and MUST stay accepted — the over-rejection guard below pins that edge.
// ---------------------------------------------------------------------------

/// The FROZEN lone-surrogate regression vectors. `(bundle, label, expect_valid)`.
fn surrogate_vectors() -> Vec<(Vec<u8>, &'static str, Option<bool>)> {
    let mut v: Vec<(Vec<u8>, &'static str, Option<bool>)> = Vec::new();

    // LOAD-BEARING (Python `_reject_lone_surrogates`): a lone HIGH surrogate
    // `\uD800` in an IGNORED anchor field. Rust's anchor `from_slice_strict`
    // pre-pass visits every string value (ignored fields included) and rejects
    // the escape → VALID | anchor-invalid | row 18 (informational). Pre-fix,
    // Python's json.loads ACCEPTED `\uD800` and read the anchor VALID | valid |
    // row 1 | tier witness-file-present — a divergence on dominant code AND row
    // AND tier. Reverting the Python surrogate check flips Python back to VALID |
    // valid | row 1 and PANICS this vector on the mismatch (the revert proof).
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_anchor_witness_raw_bytes_suffix(0, b"\"junk\":\"\\uD800\"")
            .build(),
        "lone-surrogate-anchor",
        Some(true), // both VALID | anchor-invalid | row 18 (informational)
    ));

    // LOAD-BEARING (THE CRITICAL false-VALID): a lone surrogate `\uD800` in the
    // SIGNED, member-HASHED payload of a CLOSURE erasure record (its `class`
    // field). Rust rejects the record payload at parse → CANNOT_VERIFY-malformed
    // | row 22, so the surviving withheld-erased count (0) no longer matches the
    // manifest's declared `withheld_erased: 1` → TAMPERED | tampered-scope | row
    // 10 | exit 1. Pre-fix, Python ACCEPTED the surrogate, counted the erasure as
    // withheld, and read VALID | withheld-erased | row 13 | exit 0 — declaring
    // VALID+exit-0 exactly what Rust declares TAMPERED+exit-1, the worst class of
    // divergence in a "trust-nobody" public auditor. This is a FROZEN, self-
    // signed bundle (the surrogate is baked into the signed+hashed closure
    // payload at build time — `serde_json::json!` cannot emit a lone surrogate,
    // and a post-build splice would break the member hash and be caught as row 12
    // FIRST, so the bytes are committed verbatim as a fixture). Reverting the
    // Python `_reject_lone_surrogates` check flips Python back to VALID |
    // withheld-erased | row 13 and PANICS this vector on the Rust<->Python
    // divergence — the load-bearing revert-and-fail proof for the fix.
    v.push((
        include_bytes!("fixtures/lone-surrogate-closure-record.dbev").to_vec(),
        "lone-surrogate-closure-record",
        Some(false), // both TAMPERED | tampered-scope | row 10 | exit 1
    ));

    // OVER-REJECTION GUARD: a valid surrogate PAIR `\uD83D\uDE00` (the emoji 😀,
    // U+1F600) in a record body string. json.loads COMBINES the pair into one
    // scalar (`.encode("utf-8")` succeeds), and serde_json accepts it, so BOTH
    // must read VALID | valid | row 1. If `_reject_lone_surrogates` were too eager
    // (rejecting a combined pair, not only lone surrogates) this would flip Python
    // to row 22 and PANIC — pinning the ACCEPT edge exactly where serde's is.
    v.push((
        BundleBuilder::new()
            .add_records(3)
            .with_record_raw_body(2, b"{\"s\":\"\\uD83D\\uDE00\"}")
            .build(),
        "surrogate-pair-emoji-accept",
        Some(true), // both VALID | valid | row 1 (surrogate pair combined to U+1F600)
    ));

    v
}

// ---------------------------------------------------------------------------
// The gate.
// ---------------------------------------------------------------------------
#[test]
fn rust_and_python_never_diverge() {
    let script = python_script();
    assert!(script.exists(), "python verifier not found at {}", script.display());

    if !python_available() {
        if std::env::var("EVIDENCE_FUZZ_REQUIRE").is_ok() {
            panic!("EVIDENCE_FUZZ_REQUIRE is set but python3 is not available — the \
                    Rust<->Python differential gate cannot run");
        }
        eprintln!(
            "diff_fuzz: python3 not available — SKIPPING the differential gate. \
             (Set EVIDENCE_FUZZ_REQUIRE=1 to make this a hard error, as CI does.)"
        );
        return;
    }

    let seed = env_u64("EVIDENCE_FUZZ_SEED", DEFAULT_SEED);
    let n = env_u64("EVIDENCE_FUZZ_N", 200);
    let tmp = std::env::temp_dir().join(format!("dbev-diff-fuzz-{}.dbev", std::process::id()));
    eprintln!(
        "diff_fuzz: seed={seed:#018x} N={n} per bucket (EVIDENCE_FUZZ_SEED / EVIDENCE_FUZZ_N to tune)"
    );

    let mut rng = Rng::new(seed);
    let mut checked = 0usize;

    // Frozen timestamp regression vectors first (fixed, deterministic).
    for (i, (bytes, label, expect)) in timestamp_vectors().into_iter().enumerate() {
        compare(&script, &tmp, "timestamp", seed, i, &bytes, expect);
        eprintln!("  timestamp vector ok: {label}");
        checked += 1;
    }

    // Frozen duplicate-JSON-key regression vectors (fixed, deterministic).
    for (i, (bytes, label, expect)) in dupkey_vectors().into_iter().enumerate() {
        compare(&script, &tmp, "dupkey", seed, i, &bytes, expect);
        eprintln!("  dupkey vector ok: {label}");
        checked += 1;
    }

    // Frozen serde_json-strictness regression vectors (NaN/Infinity + recursion
    // depth; fixed, deterministic).
    for (i, (bytes, label, expect)) in strictness_vectors().into_iter().enumerate() {
        compare(&script, &tmp, "strictness", seed, i, &bytes, expect);
        eprintln!("  strictness vector ok: {label}");
        checked += 1;
    }

    // Frozen uniform-strict-profile record/envelope regression vectors (the same
    // profile now applied to every JSON member, not only anchors; deterministic).
    for (i, (bytes, label, expect)) in uniform_strict_vectors().into_iter().enumerate() {
        compare(&script, &tmp, "uniform-strict", seed, i, &bytes, expect);
        eprintln!("  uniform-strict vector ok: {label}");
        checked += 1;
    }

    // Frozen lone-surrogate regression vectors (the seventh strict-profile axis:
    // a `\uXXXX` escape forming a lone UTF-16 surrogate, which json.loads accepts
    // but serde_json rejects; valid surrogate PAIRS stay accepted; deterministic).
    for (i, (bytes, label, expect)) in surrogate_vectors().into_iter().enumerate() {
        compare(&script, &tmp, "surrogate", seed, i, &bytes, expect);
        eprintln!("  surrogate vector ok: {label}");
        checked += 1;
    }

    // Bucket: valid.
    for i in 0..n {
        let bytes = gen_valid(&mut rng);
        compare(&script, &tmp, "valid", seed, i as usize, &bytes, Some(true));
        checked += 1;
    }

    // Bucket: mutated = half builder-hook (non-VALID), half raw-byte (any).
    let half = n / 2;
    for i in 0..half {
        let bytes = gen_builder_mutated(&mut rng);
        compare(&script, &tmp, "builder-mutated", seed, i as usize, &bytes, Some(false));
        checked += 1;
    }
    for i in 0..(n - half) {
        let mut bytes = gen_valid_simple(&mut rng);
        if !bytes.is_empty() {
            let off = rng.below(bytes.len() as u64) as usize;
            let delta = 1 + rng.below(254) as u8; // never a no-op change
            bytes[off] = bytes[off].wrapping_add(delta);
        }
        // Any verdict is acceptable; the invariant is only Rust==Python.
        compare(&script, &tmp, "raw-byte", seed, i as usize, &bytes, None);
        checked += 1;
    }

    let _ = std::fs::remove_file(&tmp);
    eprintln!("diff_fuzz: {checked} bundles checked, ZERO Rust<->Python divergences.");
}
