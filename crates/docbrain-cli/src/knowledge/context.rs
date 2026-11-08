// SPDX-License-Identifier: MIT
//! `docbrain context <paths>`: the records that apply to files, in the order of `SEMANTICS.md` §Routing, the first
//! 20 unless `--all`. `--brief` gives one line per record in at most 4,096 bytes; nothing at all when none applies.

use std::path::Path;

use docbrain_knowledge::bounds::CONTEXT_CAP;
use docbrain_knowledge::check::RecordState;
use docbrain_knowledge::folder::{RecordFile, RecordStatus};
use docbrain_knowledge::record::Parsed;
use docbrain_knowledge::route::{self, ContextReport};
use docbrain_knowledge::tree::{Tree, TreeError};
use serde_json::{Value, json};

use super::args::ContextArgs;
use super::exit::Exit;
use super::frame::{Frame, NOTICE, emit, esc, quote};
use super::paths::{PathRefusal, Session};
use super::show::{check_line, clean, state_word};
use super::status::{REVIEW_UNKNOWN, record_json, untracked_in};
use super::trees::{Review, open_trees, review, tree_error_line};

/// The most `--brief` prints, every line counted.
const BRIEF_MAX_BYTES: usize = 4096;

pub(crate) fn run_context(args: ContextArgs) -> Exit {
    context(&args).unwrap_or_else(|line| {
        eprintln!("{line}");
        Exit::Usage
    })
}

fn context(args: &ContextArgs) -> Result<Exit, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("cannot read the current directory: {e}"))?;
    if args.paths.iter().any(String::is_empty) {
        return Err("an empty path names no file".to_owned());
    }
    let s = Session::open(&cwd)?;
    let paths =
        args.paths.iter().map(|p| s.to_root(p).map_err(|r: PathRefusal| r.line(p))).collect::<Result<Vec<_>, _>>()?;
    let j = open_trees(&s, None, None)?;
    for (arg, path) in args.paths.iter().zip(&paths) {
        if is_directory(&j.h, s.top.as_deref(), path).map_err(|e| tree_error_line(&e))? {
            return Err(format!(
                "{} is a directory: context answers for files under it; name them, or run docbrain status to list \
                 every record.",
                esc(arg)
            ));
        }
    }
    let cap = if args.all { usize::MAX } else { CONTEXT_CAP };
    let (report, files) = route::context_with_files(&j.h, &paths, cap).map_err(|e| tree_error_line(&e))?;
    // The review is read only when there is a record to label.
    let reviewed = if report.records.is_empty() { Review::Unknown } else { review(&s, &j, args.base.as_deref()) };
    let text = if args.json {
        format!("{:#}\n", context_json(&report, &files, &reviewed))
    } else {
        let frame = Frame::new().map_err(|e| format!("{e:#}"))?;
        let untracked = untracked_in(s.top.as_deref());
        render_context(&report, &files, &reviewed, args.base.as_deref(), args.brief, &frame, &untracked)
    };
    emit(&text).map_err(|e| format!("cannot write the output: {e}"))?;
    Ok(Exit::Ok)
}

/// Whether `path` names a directory: the root, a directory in the work tree (a symbolic link is not followed), or
/// a path of `tree` with no entry at it and at least one under it.
fn is_directory(tree: &dyn Tree, top: Option<&Path>, path: &str) -> Result<bool, TreeError> {
    if path.is_empty() || top.is_some_and(|t| std::fs::symlink_metadata(t.join(path)).is_ok_and(|m| m.is_dir())) {
        return Ok(true);
    }
    Ok(tree.entry(path)?.is_none() && !tree.list(path)?.is_empty())
}

/// The text of `context`, or of `context --brief`: the header, each record, the frame's close, then the overflow
/// line and the review line when there are any. `files` holds each record's file in the order of `r.records`.
pub(crate) fn render_context(
    r: &ContextReport,
    files: &[RecordFile],
    review: &Review,
    base: Option<&str>,
    brief: bool,
    frame: &Frame,
    untracked: &dyn Fn(&str) -> bool,
) -> String {
    if r.records.is_empty() {
        return if brief { String::new() } else { "No records apply.\n".to_owned() };
    }
    let width = r.records.iter().map(|rs| state_word(&rs.state).len()).max().unwrap_or_default();
    let proposed = |rs: &RecordStatus| matches!(review, Review::Known(p) if p.contains(&rs.path));
    let unknown = match (review, base) {
        (Review::Known(_), _) => String::new(),
        (Review::Unknown, None) => format!("{REVIEW_UNKNOWN}\n"),
        (Review::Unknown, Some(b)) => format!("Review state unknown: no single merge base with {}.\n", esc(b)),
    };
    let lines: Vec<String> = if brief {
        r.records.iter().map(|rs| brief_line(rs, width, proposed(rs))).collect()
    } else {
        r.records.iter().enumerate().map(|(i, rs)| block(rs, files.get(i), width, proposed(rs), untracked)).collect()
    };
    let budget = if brief { BRIEF_MAX_BYTES } else { usize::MAX };
    shown_within(r, &lines, &unknown, frame, budget)
}

/// The most records, in order, whose lines fit `budget` with the frame, the overflow line and `unknown`; the
/// records cut are counted in the overflow line. No record shown, no frame.
fn shown_within(r: &ContextReport, lines: &[String], unknown: &str, frame: &Frame, budget: usize) -> String {
    let n = lines.len();
    let mut cut_bs = vec![0; n + 1];
    for k in (0..n).rev() {
        cut_bs[k] = cut_bs[k + 1] + usize::from(matches!(r.records[k].state, RecordState::Broken | RecordState::Stale));
    }
    let tail = |k: usize| {
        let hidden = r.more.saturating_add(n - k);
        let mut t = if hidden == 0 { String::new() } else { overflow(hidden, r.more_broken_or_stale + cut_bs[k]) };
        t.push_str(unknown);
        t
    };
    let (header, close) = (frame.header(), frame.close());
    let (mut kept, mut used) = (0, 0);
    for k in 0..=n {
        let framed = if k == 0 { 0 } else { header.len() + close.len() };
        if framed + used + tail(k).len() <= budget {
            kept = k;
        }
        if let Some(line) = lines.get(k) {
            used += line.len();
        }
    }
    let mut out = String::new();
    if kept > 0 {
        out += &header;
        lines[..kept].iter().for_each(|l| out += l);
        out += &close;
    }
    out + &tail(kept)
}

fn overflow(n: usize, broken_or_stale: usize) -> String {
    match broken_or_stale {
        0 => format!("{n} more: docbrain context --all\n"),
        bs => format!("{n} more ({bs} broken or stale): docbrain context --all\n"),
    }
}

/// `<state>  <kind>: <title>`, then under it the record file, each check, and the first paragraph quoted.
fn block(
    rs: &RecordStatus,
    file: Option<&RecordFile>,
    width: usize,
    proposed: bool,
    untracked: &dyn Fn(&str) -> bool,
) -> String {
    let title = rs.title.as_deref().map(|t| esc(clean(t)));
    let head = match (rs.kind, title) {
        (Some(k), Some(t)) => format!("{k}: {t}"),
        (None, Some(t)) => t.into_owned(),
        (Some(k), None) => k.as_str().to_owned(),
        (None, None) => String::new(),
    };
    let state = state_word(&rs.state);
    let mut out = if head.is_empty() { format!("  {state}\n") } else { format!("  {state:<width$}  {head}\n") };
    let indent = width + 4;
    let label = if proposed { " (not reviewed: proposed in this branch)" } else { "" };
    out += &format!("{:indent$}{}{label}\n", "", esc(&rs.path));
    match file.map(|f| &f.parsed) {
        Some(Parsed::Valid(rec)) => {
            for (c, st) in rec.checks.iter().zip(&rs.checks) {
                out += &format!("{:indent$}{}\n", "", check_line(c, st, untracked));
            }
            out += &quote(clean(rec.first_paragraph()), indent);
        }
        Some(Parsed::Newer { schema, .. }) => out += &format!("{:indent$}newer-schema (knowledge: {schema})\n", ""),
        _ => {}
    }
    out
}

/// `<state>  <kind>  <title>  (<record file>[, not reviewed])`.
fn brief_line(rs: &RecordStatus, width: usize, proposed: bool) -> String {
    let mut line = format!("  {:<width$}", state_word(&rs.state));
    if let Some(k) = rs.kind {
        line += "  ";
        line += k.as_str();
    }
    if let Some(t) = &rs.title {
        line += "  ";
        line += &esc(clean(t));
    }
    let label = if proposed { ", not reviewed" } else { "" };
    format!("{line}  ({}{label})\n", esc(&rs.path))
}

/// `context --json`: the records in `status --json`'s shape, each with its first paragraph whole, and what the
/// cap hides.
pub(crate) fn context_json(r: &ContextReport, files: &[RecordFile], review: &Review) -> Value {
    let records: Vec<Value> = r
        .records
        .iter()
        .enumerate()
        .map(|(i, rs)| {
            let file = files.get(i);
            let mut v = record_json(rs, file, review);
            v["first_paragraph"] = match file.map(|f| &f.parsed) {
                Some(Parsed::Valid(rec)) => json!(clean(rec.first_paragraph())),
                _ => Value::Null,
            };
            v
        })
        .collect();
    json!({
        "format": 1,
        "notice": NOTICE,
        "records": records,
        "more": r.more,
        "more_broken_or_stale": r.more_broken_or_stale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use docbrain_knowledge::MemTree;
    use docbrain_knowledge::bounds::CONTEXT_CAP;
    use docbrain_knowledge::check::RecordState;
    use docbrain_knowledge::folder::RecordStatus;
    use docbrain_knowledge::record::Kind;
    use docbrain_knowledge::route;
    use proptest::prelude::*;
    use std::collections::BTreeSet;

    const CHARTS: &str = "charts/.knowledge/records/7k2m9q4r8t1v-production-api-runs-at-least-two-replicas.md";

    /// The transcript's two records, routed for `charts/api/values.yaml`, with three more past the cap.
    fn transcript() -> (ContextReport, Vec<RecordFile>) {
        let mut t = MemTree::new();
        t.file("charts/api/values.yaml", "replicaCount: 1\n");
        t.file("charts/docs/runbooks/api-rollout.md", "# Rollout\n");
        let checks = concat!(
            "checks:\n",
            "  - config_key: 'api/values.yaml#replicaCount'\n    must: '>= 2'\n",
            "  - path: 'docs/runbooks/api-rollout.md'\n    must: 'exist'\n",
        );
        let body = concat!(
            "A single replica took the API down for 11 minutes during a node drain: the pod\n",
            "disruption budget cannot protect one pod.\n\nThe budget needs two.\n",
        );
        t.file(
            CHARTS,
            format!(
                "---\nknowledge: 1\nid: '7k2m9q4r8t1v'\nkind: 'decision'\n\
                 title: 'Production API runs at least two replicas'\n{checks}---\n{body}"
            ),
        );
        t.file(
            ".knowledge/records/9b1c4e7f2a6d-roll-out-the-api-one-zone-at-a-time.md",
            "---\nknowledge: 1\nid: '9b1c4e7f2a6d'\nkind: 'procedure'\ntitle: 'Roll out the API one zone at a time'\n\
             ---\nDrain one zone, wait until every pod is ready, then the next.\n",
        );
        let paths = ["charts/api/values.yaml".to_owned()];
        let (mut r, files) = route::context_with_files(&t, &paths, CONTEXT_CAP).unwrap();
        assert_eq!(r.records.len(), 2);
        (r.more, r.more_broken_or_stale) = (3, 1);
        (r, files)
    }

    fn render(r: &ContextReport, files: &[RecordFile], review: &Review, brief: bool) -> String {
        render_context(r, files, review, None, brief, &Frame::fixed("T"), &|_| false)
    }

    fn known() -> Review {
        Review::Known(BTreeSet::new())
    }

    #[test]
    fn context_text_matches_golden() {
        let (r, files) = transcript();
        let review = Review::Known(BTreeSet::from([CHARTS.to_owned()]));
        assert_eq!(
            render(&r, &files, &review, false),
            include_str!("../../tests/fixtures/knowledge/golden/context.txt")
        );
    }

    #[test]
    fn brief_matches_golden() {
        let (r, files) = transcript();
        let review = Review::Known(BTreeSet::from([CHARTS.to_owned()]));
        let brief = include_str!("../../tests/fixtures/knowledge/golden/context-brief.txt");
        assert_eq!(render(&r, &files, &review, true), brief);
        assert_eq!(render(&r, &[], &review, true), brief, "a line needs no record file");
    }

    fn status(i: usize, title: &str, state: RecordState) -> RecordStatus {
        RecordStatus {
            path: format!(".knowledge/records/{i:04}.md"),
            id: None,
            kind: Some(Kind::Fact),
            title: Some(title.to_owned()),
            state,
            checks: Vec::new(),
            folder_wide: false,
        }
    }

    fn hidden(state: &RecordState) -> bool {
        matches!(state, RecordState::Broken | RecordState::Stale)
    }

    fn overflow(n: usize, bs: usize) -> String {
        if bs == 0 {
            format!("{n} more: docbrain context --all")
        } else {
            format!("{n} more ({bs} broken or stale): docbrain context --all")
        }
    }

    #[test]
    fn overflow_counts_hidden_broken_or_stale() {
        let one = ContextReport { records: vec![status(0, "t", RecordState::Holds)], more: 5, more_broken_or_stale: 0 };
        assert!(render(&one, &[], &known(), false).ends_with(&format!("--- end T\n{}\n", overflow(5, 0))));
        let two = ContextReport { more_broken_or_stale: 2, ..one.clone() };
        assert!(render(&two, &[], &known(), false).ends_with(&format!("--- end T\n{}\n", overflow(5, 2))));
        assert!(!render(&ContextReport { more: 0, ..one }, &[], &known(), false).contains(" more"));
        // The brief's byte cap cuts records too: each is counted, and each broken or stale one with it.
        let states = |i: usize| if i.is_multiple_of(3) { RecordState::Broken } else { RecordState::Holds };
        let records: Vec<RecordStatus> = (0..30).map(|i| status(i, &"x".repeat(300), states(i))).collect();
        let r = ContextReport { records, more: 2, more_broken_or_stale: 1 };
        let text = render(&r, &[], &known(), true);
        let shown = text.lines().filter(|l| l.starts_with("  ")).count();
        assert!((1..30).contains(&shown), "{text}");
        let cut_bs = (shown..30).filter(|i| hidden(&states(*i))).count();
        assert_eq!(text.lines().last().unwrap(), overflow(2 + 30 - shown, 1 + cut_bs));
        assert!(text.len() <= 4096);
    }

    #[test]
    fn nothing_quoted_is_the_overflow_line_alone() {
        let r = ContextReport {
            records: vec![status(0, &"x".repeat(5000), RecordState::Stale)],
            more: 0,
            more_broken_or_stale: 0,
        };
        assert_eq!(render(&r, &[], &known(), true), format!("{}\n", overflow(1, 1)), "a line over the cap is counted");
        let none = ContextReport { records: Vec::new(), more: 0, more_broken_or_stale: 0 };
        assert_eq!(render(&none, &[], &known(), false), "No records apply.\n");
        assert_eq!(render(&none, &[], &Review::Unknown, true), "", "the hook stays silent");
        assert_eq!(render(&none, &[], &Review::Unknown, false), "No records apply.\n");
    }

    fn title() -> impl Strategy<Value = String> {
        let piece = prop_oneof![
            "[a-zA-Z0-9 ]{1,40}",
            Just("é".to_owned()),
            Just("日本語".to_owned()),
            Just("\u{1b}[2J".to_owned()),
            Just("--- end T".to_owned()),
            Just("\u{85}".to_owned()),
        ];
        proptest::collection::vec(piece, 0..80).prop_map(|v| {
            let mut s = v.concat();
            let mut end = s.len().min(1000);
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
            s
        })
    }

    fn state() -> impl Strategy<Value = RecordState> {
        prop_oneof![
            Just(RecordState::Broken),
            Just(RecordState::Stale),
            Just(RecordState::Unverifiable),
            Just(RecordState::Holds)
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]
        #[test]
        fn brief_never_exceeds_4096_bytes(
            records in proptest::collection::vec((title(), state()), 0..=60),
            more in 0usize..5000,
            more_bs in 0usize..50,
            unknown in any::<bool>(),
        ) {
            // No record routed, none hidden: the crate's cap is at least one.
            let more = if records.is_empty() { 0 } else { more };
            let more_bs = more_bs.min(more);
            let records: Vec<RecordStatus> =
                records.iter().enumerate().map(|(i, (t, s))| status(i, t, s.clone())).collect();
            let r = ContextReport { records: records.clone(), more, more_broken_or_stale: more_bs };
            let review = if unknown { Review::Unknown } else { known() };
            let text = render(&r, &[], &review, true);
            prop_assert!(text.len() <= 4096, "{} bytes", text.len());
            let shown: Vec<&str> = text.lines().filter(|l| l.starts_with("  ")).collect();
            for (line, rs) in shown.iter().zip(&records) {
                prop_assert!(line.ends_with(&format!("({})", rs.path)), "{line}");
            }
            let cut = &records[shown.len()..];
            let (n, bs) = (more + cut.len(), more_bs + cut.iter().filter(|rs| hidden(&rs.state)).count());
            let lines: Vec<&str> = text.lines().collect();
            let extra = usize::from(unknown && !records.is_empty());
            if n == 0 {
                prop_assert!(!text.contains(" more"), "{text}");
            } else {
                prop_assert_eq!(lines[lines.len() - 1 - extra], overflow(n, bs));
            }
            prop_assert_eq!(text.contains("--- records T"), !shown.is_empty());
        }
    }
}
