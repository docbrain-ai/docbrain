// SPDX-License-Identifier: MIT
//! Verdict taxonomy types + the pure aggregation rule (spec "Verdict
//! taxonomy (v2)", design doc rows 1-26). This module holds NO verification
//! logic — `verify.rs` is the only place that walks bundle bytes and
//! produces [`Finding`]s; this module only defines the vocabulary
//! ([`Verdict`], [`Finding`], [`VerdictReport`]) and the closed-world
//! precedence rule ([`aggregate`]) that turns a list of findings into one
//! verdict.
//!
//! ## Row classification (pinned here, the single source of truth other
//! modules cite by row number)
//!
//! - **Blocking TAMPERED** (rows 2,3,4,5,6,7,10,11,12): any one of these
//!   present makes the whole report TAMPERED. Per the design doc's
//!   precedence rule, TAMPERED dominates CANNOT_VERIFY.
//! - **Blocking CANNOT_VERIFY** (rows 9,14,15,16,17,21,22,26): any one of
//!   these present (and no TAMPERED finding) makes the report
//!   CANNOT_VERIFY.
//! - **Informational** (rows 1,8,13,18,19,20,23,24,25): listed in the
//!   report, never block VALID by themselves — this is the "per-anchor
//!   downgrade... chain verdict unaffected" rule (rows 18-20) generalized
//!   to every row the spec explicitly marks VALID-compatible.
//! - **Row 26 (closed-world default):** any [`Finding`] whose `row` is
//!   outside `1..=25` is itself proof of an unmapped state — `aggregate`
//!   rewrites it to `row: 26, code: "unmapped-state"` before classifying,
//!   so an unmapped state can never silently masquerade as any other row.

use serde_json::json;

/// The three closed-world verdicts (spec law 3): never collapsed, never a
/// fourth value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Valid,
    Tampered,
    CannotVerify,
}

impl Verdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Valid => "VALID",
            Verdict::Tampered => "TAMPERED",
            Verdict::CannotVerify => "CANNOT_VERIFY",
        }
    }

    /// Process exit codes, pinned by the design doc ("Verifier" section):
    /// `0 VALID / 1 TAMPERED / 2 CANNOT_VERIFY`.
    pub fn exit_code(&self) -> i32 {
        match self {
            Verdict::Valid => 0,
            Verdict::Tampered => 1,
            Verdict::CannotVerify => 2,
        }
    }
}

/// One entry per spec taxonomy row actually evaluated. `row` is the spec
/// row number 1-26 (see design doc "Verdict taxonomy"); `code` is a stable,
/// machine-matchable slug (see this module's row->code table in
/// `verify.rs`); `detail` is a human-readable explanation; `position` is
/// the record/checkpoint position this finding is about, when the finding
/// is about one specific position (`None` for bundle-wide findings like a
/// container-profile violation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub row: u8,
    pub code: &'static str,
    pub detail: String,
    pub position: Option<u64>,
}

impl Finding {
    pub fn new(row: u8, code: &'static str, detail: impl Into<String>) -> Self {
        Finding {
            row,
            code,
            detail: detail.into(),
            position: None,
        }
    }

    pub fn at(row: u8, code: &'static str, detail: impl Into<String>, position: u64) -> Self {
        Finding {
            row,
            code,
            detail: detail.into(),
            position: Some(position),
        }
    }
}

/// v1 anchor tier (R4: real TSA/QTSP token validation is Task 17's; v1
/// never grants tier >= 2). `None`: no anchor member present at all.
/// `WitnessFilePresent`: a tier-1-shaped (public witness receipt) anchor is
/// present — tier-1 was never claimed to be cryptographically validated
/// (design doc: "published observability, not CT-grade witnessing"), so
/// its mere presence is the whole claim. `TokenPresentUnvalidated`: a
/// tier-2/3-shaped (TSA/QTSP token) anchor is present but its cryptographic
/// validity is NOT checked in v1 — tier 2/3 is never granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorTier {
    None,
    WitnessFilePresent,
    TokenPresentUnvalidated,
}

/// The bundle's declared scope, echoed from the verified manifest (spec law
/// 5: every VALID output prints its declared scope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSummary {
    pub range: (u64, u64),
    pub classes: Vec<String>,
    pub spaces: Option<Vec<String>>,
}

/// The bundle's declared counts, echoed from the verified manifest (spec
/// law 5: withheld-record count printed loudly).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountsSummary {
    pub records: u64,
    pub closure: u64,
    pub withheld_erased: u64,
}

/// One time-confidence label (spec law 5): whether a wall-clock claim is
/// anchor-bounded (backed by a validated anchor) or merely self-asserted
/// (the journal's own `at` field, unverified).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeSpan {
    pub label: String,
    pub anchored: bool,
    pub at: String,
}

/// The verifier's complete output: one verdict, the dominant reason, every
/// finding actually evaluated, and the negative-space/scope/time-confidence
/// disclosures spec law 5 requires on every VALID output (rendered
/// regardless of verdict here, since a non-VALID report benefits from the
/// same honesty).
#[derive(Debug, Clone, PartialEq)]
pub struct VerdictReport {
    pub verdict: Verdict,
    pub dominant: Finding,
    pub findings: Vec<Finding>,
    pub anchor_tier: AnchorTier,
    pub scope: ScopeSummary,
    pub counts: CountsSummary,
    pub negative_space: &'static str,
    pub time_confidence: Vec<TimeSpan>,
}

/// The design doc's "What this is NOT" section, verbatim (spec, line 372-376
/// as read this session) — the negative-space text every report carries.
pub const NEGATIVE_SPACE: &str = "Not a runtime agent-action logger; not an IT-general-controls platform; not \
field-level redaction (v2, Merkle epochs + salted field commitments); not a compliance \
certification; not \"the Art 12 log\" of systems that log elsewhere.";

/// True for the taxonomy rows that, if present as a finding, make the whole
/// report TAMPERED (design doc rows 2,3,4,5,6,7,10,11,12).
fn is_blocking_tampered(row: u8) -> bool {
    matches!(row, 2 | 3 | 4 | 5 | 6 | 7 | 10 | 11 | 12)
}

/// True for the taxonomy rows that, absent any TAMPERED finding, make the
/// whole report CANNOT_VERIFY (design doc rows 9,14,15,16,17,21,22, plus 26
/// the closed-world default).
fn is_blocking_cannot_verify(row: u8) -> bool {
    matches!(row, 9 | 14 | 15 | 16 | 17 | 21 | 22 | 26)
}

/// The outcome of classifying a finding list, deliberately NOT named
/// `Verdict` and NOT itself carrying a `Valid` variant: `verify.rs`'s one
/// success exit converts `Disposition::Clean` into `Verdict::Valid` inline,
/// at the end of its own pipeline — so that a source-inspection audit of
/// `verify.rs` (see `tests/pipeline.rs::single_success_exit`) finds exactly
/// one REAL, meaningful construction of `Verdict::Valid`, not a value
/// merely passed through from this module untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// No blocking finding — every finding present (if any) is
    /// informational (rows 1,8,13,18,19,20,23,24,25).
    Clean,
    /// At least one blocking finding — carries which of the two non-VALID
    /// verdicts applies (never `Verdict::Valid`, by construction: see the
    /// `impl` below, which is the crate's only place a `Blocking` is ever
    /// built).
    Blocking(Verdict),
}

/// The pure precedence rule (design doc "Precedence" paragraph): given
/// every finding the pipeline actually produced, in check order, classify
/// them into a [`Disposition`] plus the dominant finding. TAMPERED
/// dominates CANNOT_VERIFY; dominant = first qualifying finding in check
/// order; every finding is retained in the returned list (the report lists
/// ALL findings, not just the dominant one).
///
/// Closed-world defense (row 26): any finding whose `row` is not one this
/// function recognizes (i.e. outside `1..=26`) is rewritten to
/// `row: 26, code: "unmapped-state"` BEFORE classification — this is what
/// makes an unmapped state impossible to silently misclassify as some
/// other row, and is what `tests::unmapped_row_becomes_row_26` exercises
/// directly (this codebase's closed enums make row 26 otherwise
/// unreachable through the real pipeline, by construction).
pub fn classify(findings: Vec<Finding>) -> (Disposition, Finding, Vec<Finding>) {
    let findings: Vec<Finding> = findings
        .into_iter()
        .map(|f| {
            if (1..=26).contains(&f.row) {
                f
            } else {
                Finding::new(
                    26,
                    "unmapped-state",
                    format!("no taxonomy row maps this finding: {}", f.detail),
                )
            }
        })
        .collect();

    if let Some(f) = findings.iter().find(|f| is_blocking_tampered(f.row)) {
        return (Disposition::Blocking(Verdict::Tampered), f.clone(), findings);
    }
    if let Some(f) = findings.iter().find(|f| is_blocking_cannot_verify(f.row)) {
        return (Disposition::Blocking(Verdict::CannotVerify), f.clone(), findings);
    }
    let dominant = findings
        .first()
        .cloned()
        .unwrap_or_else(|| Finding::new(1, "valid", "all checks passed"));
    (Disposition::Clean, dominant, findings)
}

/// Convenience wrapper over [`classify`] for callers (tests, future corpus
/// tooling) that just want the final [`Verdict`] without going through
/// `verify.rs`'s pipeline — NOT used by `verify_bundle` itself, which does
/// its own `Clean => Verdict::Valid` conversion inline (see module docs on
/// [`Disposition`]).
pub fn aggregate(findings: Vec<Finding>) -> (Verdict, Finding, Vec<Finding>) {
    let (disposition, dominant, findings) = classify(findings);
    let verdict = match disposition {
        Disposition::Clean => Verdict::Valid,
        Disposition::Blocking(v) => v,
    };
    (verdict, dominant, findings)
}

impl VerdictReport {
    /// Machine-readable verdict JSON enumerating every check individually
    /// (design doc "Verifier" section: "the in-toto flattening mistake,
    /// rejected") — every [`Finding`] is its own array entry, never
    /// collapsed into one summary string.
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "verdict": self.verdict.as_str(),
            "exit_code": self.verdict.exit_code(),
            "dominant": finding_json(&self.dominant),
            "findings": self.findings.iter().map(finding_json).collect::<Vec<_>>(),
            "anchor_tier": anchor_tier_str(self.anchor_tier),
            "scope": {
                "range": [self.scope.range.0, self.scope.range.1],
                "classes": self.scope.classes,
                "spaces": self.scope.spaces,
            },
            "counts": {
                "records": self.counts.records,
                "closure": self.counts.closure,
                "withheld_erased": self.counts.withheld_erased,
            },
            "negative_space": self.negative_space,
            "time_confidence": self.time_confidence.iter().map(|t| json!({
                "label": t.label,
                "anchored": t.anchored,
                "at": t.at,
            })).collect::<Vec<_>>(),
        })
    }

    /// Human-readable report text (design doc "Verifier" section: "human
    /// report + machine-readable verdict JSON").
    pub fn render_human(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Verdict: {} (row {}, {})\n",
            self.verdict.as_str(),
            self.dominant.row,
            self.dominant.code
        ));
        out.push_str(&format!("Reason: {}\n", self.dominant.detail));
        out.push_str(&format!(
            "Scope: range [{}, {}], classes {:?}\n",
            self.scope.range.0, self.scope.range.1, self.scope.classes
        ));
        out.push_str(&format!(
            "Counts: {} records, {} withheld-erased, {} closure\n",
            self.counts.records, self.counts.withheld_erased, self.counts.closure
        ));
        out.push_str(&format!("Anchor tier: {}\n", anchor_tier_str(self.anchor_tier)));
        if self.findings.len() > 1 {
            out.push_str(&format!("All findings ({}):\n", self.findings.len()));
            for f in &self.findings {
                out.push_str(&format!(
                    "  - row {} [{}]{}: {}\n",
                    f.row,
                    f.code,
                    f.position.map(|p| format!(" @position {p}")).unwrap_or_default(),
                    f.detail
                ));
            }
        }
        out.push_str("Negative space (what this does NOT prove):\n");
        out.push_str(self.negative_space);
        out.push('\n');
        out
    }
}

fn anchor_tier_str(tier: AnchorTier) -> &'static str {
    match tier {
        AnchorTier::None => "none",
        AnchorTier::WitnessFilePresent => "witness-file-present",
        AnchorTier::TokenPresentUnvalidated => "token-present-unvalidated",
    }
}

fn finding_json(f: &Finding) -> serde_json::Value {
    json!({
        "row": f.row,
        "code": f.code,
        "detail": f.detail,
        "position": f.position,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_findings_aggregate_to_valid_with_synthesized_row_1() {
        let (verdict, dominant, findings) = aggregate(vec![]);
        assert_eq!(verdict, Verdict::Valid);
        assert_eq!(dominant.row, 1);
        assert_eq!(dominant.code, "valid");
        assert!(findings.is_empty());
    }

    #[test]
    fn informational_only_findings_stay_valid_dominant_is_first_informational() {
        let findings = vec![
            Finding::at(24, "clock-anomaly", "checkpoint 20 clock went backwards", 20),
            Finding::at(25, "trivial-range", "zero-record range", 0),
        ];
        let (verdict, dominant, kept) = aggregate(findings);
        assert_eq!(verdict, Verdict::Valid);
        assert_eq!(dominant.row, 24);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn a_single_tampered_finding_makes_the_whole_report_tampered() {
        let findings = vec![Finding::at(2, "tampered-signature", "bad sig", 5)];
        let (verdict, dominant, _) = aggregate(findings);
        assert_eq!(verdict, Verdict::Tampered);
        assert_eq!(dominant.row, 2);
    }

    #[test]
    fn a_single_cannot_verify_finding_makes_the_whole_report_cannot_verify() {
        let findings = vec![Finding::at(16, "cannot-verify-unknown-key", "unknown", 5)];
        let (verdict, dominant, _) = aggregate(findings);
        assert_eq!(verdict, Verdict::CannotVerify);
        assert_eq!(dominant.row, 16);
    }

    // ---- precedence: TAMPERED dominates CANNOT_VERIFY, dominant = first
    // in check order (design doc precedence paragraph) ----

    #[test]
    fn tampered_dominates_cannot_verify_regardless_of_order() {
        let findings = vec![
            Finding::at(16, "cannot-verify-unknown-key", "unknown key", 1),
            Finding::at(2, "tampered-signature", "bad sig", 2),
        ];
        let (verdict, dominant, kept) = aggregate(findings);
        assert_eq!(verdict, Verdict::Tampered);
        assert_eq!(dominant.row, 2);
        assert_eq!(kept.len(), 2, "both findings must still be listed");
    }

    #[test]
    fn dominant_is_the_first_tampered_finding_in_check_order_not_the_worst_looking_one() {
        let findings = vec![
            Finding::at(4, "tampered-chain", "chain link mismatch", 10),
            Finding::at(2, "tampered-signature", "bad sig", 20),
        ];
        let (verdict, dominant, _) = aggregate(findings);
        assert_eq!(verdict, Verdict::Tampered);
        assert_eq!(dominant.row, 4, "row 4 was first in check order, must be dominant");
    }

    // ---- required: both a tampered record AND a malformed anchor
    // (informational anchor finding) -> TAMPERED dominant, both findings
    // present ----

    #[test]
    fn tampered_record_plus_informational_anchor_finding_is_tampered_with_both_listed() {
        let findings = vec![
            Finding::at(2, "tampered-signature", "record 5 bad sig", 5),
            Finding::new(18, "anchor-invalid", "anchor tsa-1 malformed"),
        ];
        let (verdict, dominant, kept) = aggregate(findings);
        assert_eq!(verdict, Verdict::Tampered);
        assert_eq!(dominant.row, 2);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().any(|f| f.row == 18));
    }

    // ---- row 26 closed-world default ----

    #[test]
    fn unmapped_row_becomes_row_26() {
        // No real pipeline path can construct a Finding with a row outside
        // 1..=25 (every enum variant this crate defines is exhaustively
        // matched into one of the 25 named rows) — this test proves the
        // DEFENSIVE catch-all itself works, directly, since the real
        // pipeline structurally cannot reach it (that absence IS the
        // guarantee: the closed-world default is a backstop that Rust's
        // exhaustive matching makes unreachable in practice).
        let findings = vec![Finding::new(99, "bogus-row", "not a real taxonomy row")];
        let (verdict, dominant, kept) = aggregate(findings);
        assert_eq!(verdict, Verdict::CannotVerify);
        assert_eq!(dominant.row, 26);
        assert_eq!(dominant.code, "unmapped-state");
        assert_eq!(kept[0].row, 26);
    }

    #[test]
    fn row_zero_also_becomes_row_26() {
        let findings = vec![Finding::new(0, "also-bogus", "zero is not a row")];
        let (verdict, dominant, _) = aggregate(findings);
        assert_eq!(verdict, Verdict::CannotVerify);
        assert_eq!(dominant.row, 26);
    }

    #[test]
    fn row_26_itself_is_a_recognized_cannot_verify_row_not_double_wrapped() {
        let findings = vec![Finding::new(26, "unmapped-state", "already row 26")];
        let (verdict, dominant, kept) = aggregate(findings);
        assert_eq!(verdict, Verdict::CannotVerify);
        assert_eq!(dominant.row, 26);
        assert_eq!(kept[0].detail, "already row 26");
    }

    #[test]
    fn to_json_lists_every_finding_not_just_dominant() {
        let findings = vec![
            Finding::at(2, "tampered-signature", "bad sig", 5),
            Finding::new(18, "anchor-invalid", "bad anchor"),
        ];
        let (verdict, dominant, kept) = aggregate(findings);
        let report = VerdictReport {
            verdict,
            dominant,
            findings: kept,
            anchor_tier: AnchorTier::None,
            scope: ScopeSummary {
                range: (0, 10),
                classes: vec!["fragment".to_string()],
                spaces: None,
            },
            counts: CountsSummary {
                records: 10,
                closure: 0,
                withheld_erased: 0,
            },
            negative_space: NEGATIVE_SPACE,
            time_confidence: vec![],
        };
        let json = report.to_json();
        assert_eq!(json["verdict"], "TAMPERED");
        assert_eq!(json["exit_code"], 1);
        assert_eq!(json["findings"].as_array().unwrap().len(), 2);
        assert_eq!(json["dominant"]["row"], 2);
    }

    #[test]
    fn exit_codes_match_the_spec() {
        assert_eq!(Verdict::Valid.exit_code(), 0);
        assert_eq!(Verdict::Tampered.exit_code(), 1);
        assert_eq!(Verdict::CannotVerify.exit_code(), 2);
    }
}
