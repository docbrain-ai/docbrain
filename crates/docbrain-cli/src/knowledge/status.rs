// SPDX-License-Identifier: MIT
//! `docbrain status [<rev>]`: every record in H and its state, the folders' own problems, and with `--fail-on
//! broken` an exit that fails on a `must` that does not hold.

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use docbrain_knowledge::bounds::RECORDS_PER_PASS;
use docbrain_knowledge::check::{CheckState, RecordState};
use docbrain_knowledge::folder::{self, RecordFile, RecordStatus, StatusReport};
use docbrain_knowledge::path::glob_match;
use docbrain_knowledge::record::{Check, Parsed, Verb};
use docbrain_knowledge::{Fail, Tree, TreeError};
use serde_json::{Value, json};

use super::args::{FailOn, StatusArgs};
use super::exit::Exit;
use super::frame::{Frame, NOTICE, emit, esc};
use super::gate::run_gate;
use super::paths::Session;
use super::show::{anchor_path, anchored_files, check_line, check_word, clean, shown, shown_anchor, state_word};
use super::trees::{Review, open_trees, review, tree_error_line};

/// Said once after the summary when no reviewed base is known: no record is shown as reviewed.
pub(crate) const REVIEW_UNKNOWN: &str = "Review state unknown: no single merge base with refs/remotes/origin/HEAD.";

/// The width of the label column inside the frame.
const LABEL: usize = 14;
/// A record's further lines start under its file.
const MORE: &str = "                ";

pub(crate) fn run_status(args: StatusArgs) -> Exit {
    status(&args).unwrap_or_else(|line| {
        eprintln!("{line}");
        Exit::Usage
    })
}

fn status(args: &StatusArgs) -> Result<Exit, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("cannot read the current directory: {e}"))?;
    let s = Session::open(&cwd)?;
    let spec = Pathspec::parse(&s, &args.pathspec)?;
    if let Some(base) = &args.base {
        return run_gate(&s, args, base, &spec);
    }
    let j = open_trees(&s, args.rev.as_deref(), None)?;
    let report = folder::status(&j.h).map_err(|e| tree_error_line(&e))?;
    let reviewed = review(&s, &j, None);
    let (inside, outside) = broken_musts(&report, &spec);
    let text = if args.json {
        let v = status_json(&report, &reviewed, &j.h).map_err(|e| tree_error_line(&e))?;
        format!("{v:#}\n")
    } else {
        let frame = Frame::new().map_err(|e| format!("{e:#}"))?;
        let untracked = untracked_in(if args.rev.is_none() { s.top.as_deref() } else { None });
        let mut text = render_status(&report, &reviewed, &j.h, &untracked, &frame).map_err(|e| tree_error_line(&e))?;
        if args.fail_on.is_some() {
            text += &verdict(inside, outside, &spec);
        }
        text
    };
    emit(&text).map_err(|e| format!("cannot write the output: {e}"))?;
    Ok(if args.fail_on == Some(FailOn::Broken) && inside > 0 { Exit::Fail } else { Exit::Ok })
}

/// The paths after `--`: each converted as a path argument; a finding counts when its anchored file is one
/// of them or lies under one, or matches one holding `*` or `?`. None given: every file.
pub(crate) struct Pathspec {
    specs: Vec<Spec>,
    typed: Vec<String>,
}

enum Spec {
    Under(String),
    Glob(String),
}

impl Pathspec {
    pub(crate) fn parse(s: &Session, args: &[String]) -> Result<Pathspec, String> {
        let mut specs = Vec::with_capacity(args.len());
        for a in args {
            if a.starts_with(':') {
                return Err(format!("{a}: pathspec magic is not supported; name a path or a glob"));
            }
            let root = s.to_root(a).map_err(|r| r.line(a))?;
            specs.push(if root.contains(['*', '?']) { Spec::Glob(root) } else { Spec::Under(root) });
        }
        Ok(Pathspec { specs, typed: args.to_vec() })
    }

    /// Whether the file of `anchor` is in the pathspec. A file name may hold `#` as a key path's separator does, so
    /// the whole anchor and the text before each `#` are tried: a finding is never taken outside by a `#`.
    pub(crate) fn matches(&self, anchor: &str) -> bool {
        anchor.match_indices('#').map(|(i, _)| &anchor[..i]).chain([anchor]).any(|file| self.matches_file(file))
    }

    /// Whether `file`, a root path, is in the pathspec.
    pub(crate) fn matches_file(&self, file: &str) -> bool {
        self.specs.is_empty()
            || self.specs.iter().any(|s| match s {
                Spec::Under(p) => {
                    p.is_empty() || file.strip_prefix(p.as_str()).is_some_and(|r| r.is_empty() || r.starts_with('/'))
                }
                Spec::Glob(g) => glob_match(g, file),
            })
    }

    /// The pathspec as typed, for the line that counts what lies outside it.
    pub(crate) fn typed(&self) -> String {
        self.typed.join(", ")
    }
}

/// Live records' `must` checks that are broken, once per record and anchor: those whose anchored file (a layered
/// key's first) is in the pathspec, and those outside it.
fn broken_musts(r: &StatusReport, spec: &Pathspec) -> (usize, usize) {
    let files = record_files(r);
    let mut seen = BTreeSet::new();
    let (mut inside, mut outside) = (0, 0);
    for rs in &r.records {
        let Some(Parsed::Valid(rec)) = files.get(rs.path.as_str()).map(|f| &f.parsed) else {
            continue;
        };
        for (c, st) in rec.checks.iter().zip(&rs.checks) {
            if c.verb != Verb::Must || !matches!(st, CheckState::Broken(_)) || !seen.insert((&rs.path, anchor_path(c)))
            {
                continue;
            }
            if spec.matches_file(anchored_files(c).first().copied().unwrap_or_default()) {
                inside += 1;
            } else {
                outside += 1;
            }
        }
    }
    (inside, outside)
}

pub(crate) fn verdict(inside: usize, outside: usize, spec: &Pathspec) -> String {
    let findings = |n: usize| {
        if n == 1 { "1 broken finding".to_owned() } else { format!("{n} broken findings") }
    };
    let mut out = if inside > 0 {
        format!("Fails: {} (--fail-on broken).\n", findings(inside))
    } else {
        format!("Passes: {} (--fail-on broken).\n", findings(0))
    };
    if outside > 0 {
        out += &format!("{} outside {}: reported, not blocking.\n", findings(outside), spec.typed());
    }
    out
}

fn record_files(r: &StatusReport) -> BTreeMap<&str, &RecordFile> {
    r.folders.iter().flat_map(|f| &f.records).map(|f| (f.path.as_str(), f)).collect()
}

/// `broken` and `stale` first as one group, then the rest; each group by record file in byte order.
fn ordered(r: &StatusReport) -> Vec<&RecordStatus> {
    let mut v: Vec<&RecordStatus> = r.records.iter().collect();
    v.sort_by_key(|rs| (!matches!(rs.state, RecordState::Broken | RecordState::Stale), rs.path.as_str()));
    v
}

/// The text of `status`: the header, every record, the folders' problems, the frame's close and the summary; one
/// line when there is nothing to quote.
pub(crate) fn render_status(
    r: &StatusReport,
    review: &Review,
    tree: &dyn Tree,
    untracked: &dyn Fn(&str) -> bool,
    frame: &Frame,
) -> Result<String, TreeError> {
    let files = record_files(r);
    let records = ordered(r);
    let notes = notes(r, review, tree, &files)?;
    if records.is_empty() && notes.is_empty() {
        return Ok("No records.\n".to_owned());
    }
    let mut out = frame.header();
    for rs in &records {
        out += &record_block(rs, files.get(rs.path.as_str()).copied(), untracked);
    }
    out += &notes;
    out += &frame.close();
    out += &summary(&records);
    if matches!(review, Review::Unknown) && !records.is_empty() {
        out += REVIEW_UNKNOWN;
        out.push('\n');
    }
    Ok(out)
}

fn head(label: &str, rest: &str) -> String {
    format!("  {label:<LABEL$}{rest}\n")
}

fn record_block(rs: &RecordStatus, file: Option<&RecordFile>, untracked: &dyn Fn(&str) -> bool) -> String {
    let path = esc(&rs.path);
    let mut out = match &rs.state {
        RecordState::Superseded { by } => head("superseded", &format!("{path} by {}", by.as_str())),
        st => head(state_word(st), &path),
    };
    let mut more = |line: &str| {
        out.push_str(MORE);
        out.push_str(line);
        out.push('\n');
    };
    let title = rs.title.as_deref().map(|t| esc(clean(t)));
    match (rs.kind, title) {
        (Some(k), Some(t)) => more(&format!("{k}: {t}")),
        (None, Some(t)) => more(&t),
        (Some(k), None) => more(k.as_str()),
        (None, None) => {}
    }
    match (&rs.state, file.map(|f| &f.parsed)) {
        (RecordState::Invalid { reason, line, col }, _) => more(&format!("{} at {line}:{col}", reason.code())),
        (RecordState::Superseded { .. }, _) => {}
        (_, Some(Parsed::Newer { schema, .. })) => more(&format!("newer-schema (knowledge: {schema})")),
        (_, Some(Parsed::Valid(rec))) => {
            for (c, st) in rec.checks.iter().zip(&rs.checks) {
                more(&check_line(c, st, untracked));
            }
        }
        _ => {}
    }
    out
}

/// The lines after the records: folder-wide records, records whose `applies_to` matches no file, records this
/// branch proposes, records not read, entries that are not records, duplicate ids, `supersedes` problems, cycles.
fn notes(
    r: &StatusReport,
    review: &Review,
    tree: &dyn Tree,
    files: &BTreeMap<&str, &RecordFile>,
) -> Result<String, TreeError> {
    let mut out = String::new();
    for rs in r.records.iter().filter(|rs| rs.folder_wide) {
        out += &head("folder-wide", &format!("{}: context for every file in its folder", esc(&rs.path)));
    }
    for path in matches_nothing(r, tree)? {
        out += &head("no match", &format!("{}: applies_to matches no file", esc(path)));
    }
    if let Review::Known(proposed) = review {
        for path in proposed.iter().filter(|p| files.contains_key(p.as_str())) {
            out += &head("not reviewed", &format!("{}: proposed in this branch", esc(path)));
        }
    }
    for (scope, n) in &r.over_bound {
        let folder = esc(&knowledge_dir(scope)).into_owned();
        let records = if *n == 1 { "record" } else { "records" };
        out += &head(
            "not read",
            &format!("{folder}: {} {records} over {} in one pass", thousands(*n), thousands(RECORDS_PER_PASS)),
        );
    }
    for f in &r.folders {
        for e in &f.ignored {
            out += &head("not a record", &esc(e));
        }
        for (id, paths) in &f.duplicates {
            let paths = paths.iter().map(|p| esc(p)).collect::<Vec<_>>().join(", ");
            out += &head("duplicate id", &format!("{}: {paths}", id.as_str()));
        }
        for (path, problem) in &f.supersede_problems {
            out += &head("supersedes", &format!("{}: {}", esc(path), json!(problem).as_str().unwrap_or_default()));
        }
        for cycle in &f.cycles {
            out += &head("cycle", &cycle.iter().map(|id| id.as_str()).collect::<Vec<_>>().join(", "));
        }
    }
    Ok(out)
}

/// The `.knowledge` folder of a scope, from the root.
fn knowledge_dir(scope: &str) -> String {
    if scope.is_empty() { ".knowledge".to_owned() } else { format!("{scope}/.knowledge") }
}

/// Live records none of whose `applies_to` globs matches an entry of the tree, in byte order (edge case 47).
fn matches_nothing<'a>(r: &'a StatusReport, tree: &dyn Tree) -> Result<Vec<&'a str>, TreeError> {
    let mut listed: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut out = Vec::new();
    for rf in r.folders.iter().flat_map(|f| &f.records) {
        let globs = match &rf.parsed {
            _ if rf.superseded_by.is_some() => continue,
            Parsed::Valid(rec) => &rec.applies_to,
            Parsed::Newer { applies_to, .. } => applies_to,
            Parsed::Invalid { .. } => continue,
        };
        let mut any = false;
        for g in globs {
            any = if g.is_literal() {
                tree.entry(&g.root)?.is_some()
            } else {
                let dir = g.literal_dir();
                if !listed.contains_key(dir) {
                    listed.insert(dir, tree.list(dir)?.into_iter().map(|(p, _)| p).collect());
                }
                listed[dir].iter().any(|p| g.matches(p))
            };
            if any {
                break;
            }
        }
        if !any {
            out.push(rf.path.as_str());
        }
    }
    out.sort_unstable();
    Ok(out)
}

fn summary(records: &[&RecordStatus]) -> String {
    let n = |f: fn(&RecordState) -> bool| records.iter().filter(|r| f(&r.state)).count();
    format!(
        "{} {}: {} broken, {} stale, {} unverifiable, {} holds, {} invalid, {} superseded.\n",
        records.len(),
        if records.len() == 1 { "record" } else { "records" },
        n(|s| matches!(s, RecordState::Broken)),
        n(|s| matches!(s, RecordState::Stale)),
        n(|s| matches!(s, RecordState::Unverifiable)),
        n(|s| matches!(s, RecordState::Holds)),
        n(|s| matches!(s, RecordState::Invalid { .. })),
        n(|s| matches!(s, RecordState::Superseded { .. })),
    )
}

fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(d);
    }
    out
}

/// `status --json`: the same records in the same order, every value whole and unescaped, in the gate's field names.
pub(crate) fn status_json(r: &StatusReport, review: &Review, tree: &dyn Tree) -> Result<Value, TreeError> {
    let files = record_files(r);
    let records: Vec<Value> =
        ordered(r).into_iter().map(|rs| record_json(rs, files.get(rs.path.as_str()).copied(), review)).collect();
    let folders = || r.folders.iter();
    Ok(json!({
        "format": 1,
        "notice": NOTICE,
        "records": records,
        "folder_wide": r.records.iter().filter(|rs| rs.folder_wide).map(|rs| &rs.path).collect::<Vec<_>>(),
        "matches_nothing": matches_nothing(r, tree)?,
        "over_bound": r.over_bound.iter().map(|(scope, count)| json!({"folder": knowledge_dir(scope), "count": count})).collect::<Vec<_>>(),
        "ignored": folders().flat_map(|f| &f.ignored).collect::<Vec<_>>(),
        "duplicates": folders()
            .flat_map(|f| &f.duplicates)
            .map(|(id, paths)| json!({"id": id.as_str(), "record_paths": paths}))
            .collect::<Vec<_>>(),
        "supersede_problems": folders()
            .flat_map(|f| &f.supersede_problems)
            .map(|(path, problem)| json!({"record_path": path, "problem": problem}))
            .collect::<Vec<_>>(),
        "cycles": folders()
            .flat_map(|f| &f.cycles)
            .map(|c| c.iter().map(|id| id.as_str()).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
    }))
}

pub(crate) fn record_json(rs: &RecordStatus, file: Option<&RecordFile>, review: &Review) -> Value {
    let checks: Vec<Value> = match file.map(|f| &f.parsed) {
        Some(Parsed::Valid(rec)) => rec.checks.iter().zip(&rs.checks).map(|(c, st)| check_json(c, st)).collect(),
        _ => Vec::new(),
    };
    let reviewed = match review {
        Review::Unknown => "unknown",
        Review::Known(proposed) if proposed.contains(&rs.path) => "not-reviewed",
        Review::Known(_) => "reviewed",
    };
    let mut o = json!({
        "record_path": rs.path,
        "id": rs.id.as_ref().map(|id| id.as_str()),
        "kind": rs.kind.map(|k| k.as_str()),
        "title": rs.title.as_deref().map(clean),
        "state": state_word(&rs.state),
        "review": reviewed,
        "checks": checks,
    });
    match (&rs.state, file.map(|f| &f.parsed)) {
        (RecordState::Invalid { reason, line, col }, _) => {
            o["reason"] = json!(reason.code());
            o["line"] = json!(line);
            o["col"] = json!(col);
        }
        (RecordState::Superseded { by }, _) => o["by"] = json!(by.as_str()),
        (_, Some(Parsed::Newer { .. })) => o["reason"] = json!("newer-schema"),
        _ => {}
    }
    o
}

fn check_json(c: &Check, st: &CheckState) -> Value {
    let mut o = json!({"verb": c.verb.as_str(), "anchor": shown_anchor(&anchor_path(c)), "expect": clean(&c.expect.to_string()), "state": check_word(st)});
    match st {
        CheckState::Holds(Some(v)) => o["shown"] = json!(shown(v)),
        CheckState::Holds(None) => {}
        CheckState::Broken(f) | CheckState::Stale(f) => {
            o["reason"] = json!(f.code());
            if let Fail::False { actual } = f {
                o["shown"] = json!(shown(actual));
            }
        }
        CheckState::Unverifiable(u) => o["reason"] = json!(u.code()),
    }
    o
}

/// Whether a path (or glob) exists in the working tree under `top` without being tracked or ignored. Asked only
/// for a missing file, so git lists the untracked files once, on the first question; without a working tree
/// (a `<rev>`), never.
pub(crate) fn untracked_in(top: Option<&Path>) -> impl Fn(&str) -> bool + '_ {
    let files: OnceCell<BTreeSet<String>> = OnceCell::new();
    move |p: &str| {
        let Some(top) = top else { return false };
        let set = files.get_or_init(|| untracked_files(top));
        if p.contains(['*', '?']) {
            return set.iter().any(|f| glob_match(p, f));
        }
        let under = format!("{p}/");
        set.contains(p) || set.range(under.clone()..).next().is_some_and(|f| f.starts_with(&under))
    }
}

fn untracked_files(top: &Path) -> BTreeSet<String> {
    let out = Command::new("git")
        .current_dir(top)
        .env("LC_ALL", "C")
        .args(["ls-files", "-z", "--others", "--exclude-standard"])
        .output();
    match out {
        Ok(o) if o.status.success() => o
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .filter_map(|p| std::str::from_utf8(p).ok())
            .map(str::to_owned)
            .collect(),
        _ => BTreeSet::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::testrepo::Repo;
    use docbrain_knowledge::MemTree;

    const ROOT: &str = ".knowledge/records/";
    const CHARTS: &str = "charts/.knowledge/records/7k2m9q4r8t1v-production-api-runs-at-least-two-replicas.md";

    fn rec(id: &str, kind: &str, title: &str, rest: &str) -> String {
        format!("---\nknowledge: 1\nid: '{id}'\nkind: '{kind}'\ntitle: '{title}'\n{rest}---\nWhy.\n")
    }

    fn render(t: &MemTree, review: &Review) -> String {
        let r = folder::status(t).unwrap();
        render_status(&r, review, t, &|_| false, &Frame::fixed("T")).unwrap()
    }

    /// The frame's interior: the lines between its open and close lines.
    fn interior(text: &str) -> Vec<&str> {
        let lines: Vec<&str> = text.lines().collect();
        let open = lines.iter().position(|l| *l == "--- records T").expect("a frame");
        let close = lines.iter().position(|l| *l == "--- end T").expect("a frame");
        lines[open + 1..close].to_vec()
    }

    #[test]
    fn status_text_matches_golden() {
        let mut t = MemTree::new();
        t.file("charts/api/values.yaml", "replicaCount: 1\nimage:\n  tag: '1.3'\n");
        t.file("charts/docs/runbooks/api-rollout.md", "# Rollout\n");
        let checks = "checks:\n  - config_key: 'api/values.yaml#replicaCount'\n    must: '>= 2'\n  - path: 'docs/runbooks/api-rollout.md'\n    must: 'exist'\n";
        t.file(CHARTS, rec("7k2m9q4r8t1v", "decision", "Production API runs at least two replicas", checks));
        let tag = "checks:\n  - config_key: 'charts/api/values.yaml#image.tag'\n    assumes: '>= 1.29'\n";
        t.file(
            &format!("{ROOT}5d8e2c1b9a3f-the-api-image-is-pinned-to-a-release.md"),
            rec("5d8e2c1b9a3f", "fact", "The API image is pinned to a release", tag),
        );
        t.file(
            &format!("{ROOT}9b1c4e7f2a6d-roll-out-the-api-one-zone-at-a-time.md"),
            rec("9b1c4e7f2a6d", "procedure", "Roll out the API one zone at a time", ""),
        );
        t.file(
            &format!("{ROOT}3f6h8j2k4m5n-retry-policy.md"),
            "---\nknowledge: 1\nid: '3f6h8j2k4m5n'\nkind: 'decision'\ntitle: Retry policy\n---\nWhy.\n",
        );
        t.file(
            &format!("{ROOT}3c4d5e6f7g8h-billing-queue.md"),
            rec("3c4d5e6f7g8h", "fact", "Billing jobs retry through one queue", "applies_to: ['billing/**']\n"),
        );
        let review = Review::Known(BTreeSet::from([CHARTS.to_owned()]));
        assert_eq!(render(&t, &review), include_str!("../../tests/fixtures/knowledge/golden/status.txt"));
    }

    #[test]
    fn over_bound_is_one_line_per_folder() {
        let r = StatusReport {
            records: Vec::new(),
            folders: Vec::new(),
            over_bound: vec![(String::new(), 5), ("charts".into(), 1)],
        };
        let text = render_status(&r, &Review::Unknown, &MemTree::new(), &|_| false, &Frame::fixed("T")).unwrap();
        assert_eq!(
            interior(&text),
            [
                "  not read      .knowledge: 5 records over 100,000 in one pass",
                "  not read      charts/.knowledge: 1 record over 100,000 in one pass",
            ]
        );
        assert!(
            text.ends_with(
                "--- end T\n0 records: 0 broken, 0 stale, 0 unverifiable, 0 holds, 0 invalid, 0 superseded.\n"
            ),
            "{text}"
        );
        let v = status_json(&r, &Review::Unknown, &MemTree::new()).unwrap();
        assert_eq!(
            v["over_bound"],
            json!([{"folder": ".knowledge", "count": 5}, {"folder": "charts/.knowledge", "count": 1}])
        );
    }

    #[test]
    fn folder_wide_records_are_named() {
        let mut t = MemTree::new();
        t.file("svc/a.rs", "fn a() {}\n");
        t.file(
            &format!("{ROOT}2a3b4c5d6e7f-everything.md"),
            rec("2a3b4c5d6e7f", "fact", "Everything", "applies_to: ['**']\n"),
        );
        t.file(&format!("{ROOT}3c4d5e6f7g8h-no-checks.md"), rec("3c4d5e6f7g8h", "fact", "No checks", ""));
        t.file(
            &format!("{ROOT}5d8e2c1b9a3f-narrow.md"),
            rec("5d8e2c1b9a3f", "fact", "Narrow", "applies_to: ['svc/**']\n"),
        );
        t.file(&format!("{ROOT}7k2m9q4r8t1v-old.md"), rec("7k2m9q4r8t1v", "fact", "Old", "applies_to: ['**']\n"));
        t.file(
            &format!("{ROOT}9b1c4e7f2a6d-new.md"),
            rec("9b1c4e7f2a6d", "fact", "New", "supersedes: '7k2m9q4r8t1v'\napplies_to: ['svc/a.rs']\n"),
        );
        t.file("svc/.knowledge/records/4d5e6f7g8h9j-svc.md", rec("4d5e6f7g8h9j", "fact", "Svc", ""));
        let wide: Vec<String> = interior(&render(&t, &Review::Unknown))
            .into_iter()
            .filter(|l| l.starts_with("  folder-wide"))
            .map(str::to_owned)
            .collect();
        assert_eq!(
            wide,
            [
                "  folder-wide   .knowledge/records/2a3b4c5d6e7f-everything.md: context for every file in its folder",
                "  folder-wide   .knowledge/records/3c4d5e6f7g8h-no-checks.md: context for every file in its folder",
                "  folder-wide   svc/.knowledge/records/4d5e6f7g8h9j-svc.md: context for every file in its folder",
            ]
        );
    }

    #[test]
    fn applies_to_matching_nothing_is_named() {
        let mut t = MemTree::new();
        t.file("src/a.rs", "fn a() {}\n");
        let applies = |globs: &str| format!("applies_to: [{globs}]\n");
        t.file(
            &format!("{ROOT}2a3b4c5d6e7f-billing.md"),
            rec("2a3b4c5d6e7f", "fact", "Billing", &applies("'billing/**'")),
        );
        t.file(&format!("{ROOT}3c4d5e6f7g8h-empty.md"), rec("3c4d5e6f7g8h", "fact", "Empty", &applies("")));
        t.file(&format!("{ROOT}4d5e6f7g8h9j-glob.md"), rec("4d5e6f7g8h9j", "fact", "Glob", &applies("'src/*.rs'")));
        t.file(&format!("{ROOT}5d8e2c1b9a3f-file.md"), rec("5d8e2c1b9a3f", "fact", "File", &applies("'src/a.rs'")));
        t.file(&format!("{ROOT}6e7f8g9h0j1k-dir.md"), rec("6e7f8g9h0j1k", "fact", "Dir", &applies("'src'")));
        t.file(&format!("{ROOT}7k2m9q4r8t1v-old.md"), rec("7k2m9q4r8t1v", "fact", "Old", &applies("'gone/**'")));
        t.file(
            &format!("{ROOT}8m9n0p1q2r3s-new.md"),
            rec("8m9n0p1q2r3s", "fact", "New", "supersedes: '7k2m9q4r8t1v'\napplies_to: ['src/a.rs']\n"),
        );
        t.file(
            &format!("{ROOT}9b1c4e7f2a6d-next.md"),
            "---\nknowledge: 2\nid: '9b1c4e7f2a6d'\nkind: 'fact'\ntitle: 'Next'\napplies_to: ['web/**']\n---\nb\n",
        );
        t.file(
            &format!("{ROOT}9c2d5e6f7g8h-bad.md"),
            "---\nknowledge: 1\nid: '9c2d5e6f7g8h'\nkind: 'fact'\ntitle: Bad\napplies_to: ['gone/**']\n---\nb\n",
        );
        let none: Vec<String> = interior(&render(&t, &Review::Unknown))
            .into_iter()
            .filter(|l| l.starts_with("  no match"))
            .map(str::to_owned)
            .collect();
        let line = |f: &str| format!("  no match      {ROOT}{f}: applies_to matches no file");
        assert_eq!(
            none,
            [
                line("2a3b4c5d6e7f-billing.md"),
                line("3c4d5e6f7g8h-empty.md"),
                line("6e7f8g9h0j1k-dir.md"),
                line("9b1c4e7f2a6d-next.md")
            ]
        );
    }

    #[test]
    fn broken_and_stale_come_first() {
        let mut t = MemTree::new();
        let check = |verb: &str, path: &str| format!("checks:\n  - path: '{path}'\n    {verb}: 'exist'\n");
        t.file("here.md", "x\n");
        t.file(&format!("{ROOT}a.md"), rec("2a3b4c5d6e7f", "fact", "Holds", &check("must", "here.md")));
        t.file(&format!("{ROOT}b.md"), rec("3c4d5e6f7g8h", "fact", "Stale", &check("assumes", "gone.md")));
        t.file(&format!("{ROOT}c.md"), rec("4d5e6f7g8h9j", "fact", "Broken", &check("must", "gone.md")));
        t.file(&format!("{ROOT}d.md"), "---\nknowledge: 1\n---\nb\n");
        t.file(&format!("{ROOT}e.md"), rec("5d8e2c1b9a3f", "fact", "Old", &check("must", "gone.md")));
        t.file(
            &format!("{ROOT}f.md"),
            rec("6e7f8g9h0j1k", "fact", "New", &format!("supersedes: '5d8e2c1b9a3f'\n{}", check("must", "here.md"))),
        );
        t.file(
            &format!("{ROOT}0.md"),
            rec("7k2m9q4r8t1v", "fact", "Unverifiable", "checks:\n  - config_key: 'here.md#a'\n    must: 'between'\n"),
        );
        let heads: Vec<String> = interior(&render(&t, &Review::Unknown))
            .into_iter()
            .filter(|l| !l.starts_with("   ") && !l.starts_with("  folder-wide") && !l.starts_with("  no match"))
            .map(str::to_owned)
            .collect();
        assert_eq!(
            heads,
            [
                format!("  stale         {ROOT}b.md"),
                format!("  broken        {ROOT}c.md"),
                format!("  unverifiable  {ROOT}0.md"),
                format!("  holds         {ROOT}a.md"),
                format!("  invalid       {ROOT}d.md"),
                format!("  superseded    {ROOT}e.md by 6e7f8g9h0j1k"),
                format!("  holds         {ROOT}f.md"),
            ]
        );
    }

    #[test]
    fn a_replaced_record_in_a_newer_format_says_only_what_replaced_it() {
        let mut t = MemTree::new();
        t.file(
            &format!("{ROOT}9b1c4e7f2a6d-next.md"),
            "---\nknowledge: 2\nid: '9b1c4e7f2a6d'\nkind: 'fact'\ntitle: 'Next'\napplies_to: ['**']\n---\nb\n",
        );
        t.file(
            &format!("{ROOT}2a3b4c5d6e7f-now.md"),
            rec("2a3b4c5d6e7f", "fact", "Now", "supersedes: '9b1c4e7f2a6d'\n"),
        );
        let text = render(&t, &Review::Unknown);
        let lines = interior(&text);
        let at = lines
            .iter()
            .position(|l| *l == format!("  superseded    {ROOT}9b1c4e7f2a6d-next.md by 2a3b4c5d6e7f"))
            .expect("the replaced record's line");
        assert_eq!(lines[at + 1].trim_start(), "fact: Next", "{lines:?}");
        assert!(!lines.iter().any(|l| l.contains("newer-schema")), "{lines:?}");
    }

    #[test]
    fn folder_problems_are_one_line_each() {
        let mut t = MemTree::new();
        t.file(&format!("{ROOT}a.md"), rec("2a3b4c5d6e7f", "fact", "A", ""));
        t.file(&format!("{ROOT}b.md"), rec("2a3b4c5d6e7f", "fact", "B", ""));
        t.file(&format!("{ROOT}c.md"), rec("3c4d5e6f7g8h", "fact", "C", "supersedes: '9z9z9z9z9z9z'\n"));
        t.file(&format!("{ROOT}d.md"), rec("4d5e6f7g8h9j", "fact", "D", "supersedes: '4d5e6f7g8h9j'\n"));
        t.file(&format!("{ROOT}e.md"), rec("5d8e2c1b9a3f", "fact", "E", "supersedes: '6e7f8g9h0j1k'\n"));
        t.file(&format!("{ROOT}f.md"), rec("6e7f8g9h0j1k", "fact", "F", "supersedes: '5d8e2c1b9a3f'\n"));
        t.file(&format!("{ROOT}README.txt"), "x");
        t.file(&format!("{ROOT}g\u{1b}[2J.md"), rec("7k2m9q4r8t1v", "fact", "G", ""));
        let r = folder::status(&t).unwrap();
        let text = render(&t, &Review::Unknown);
        let notes: Vec<&str> = interior(&text)
            .into_iter()
            .filter(|l| !l.starts_with("   ") && !l.starts_with("  holds") && !l.starts_with("  folder-wide"))
            .collect();
        assert_eq!(
            notes,
            [
                format!("  not a record  {ROOT}README.txt"),
                format!("  duplicate id  2a3b4c5d6e7f: {ROOT}a.md, {ROOT}b.md"),
                format!("  supersedes    {ROOT}c.md: not-in-folder"),
                format!("  supersedes    {ROOT}d.md: itself"),
                "  cycle         5d8e2c1b9a3f, 6e7f8g9h0j1k".to_owned(),
            ]
        );
        assert!(text.contains(&format!("  holds         {ROOT}g\\x{{1b}}[2J.md\n")), "{text}");
        let v = status_json(&r, &Review::Unknown, &t).unwrap();
        assert_eq!(v["ignored"], json!([format!("{ROOT}README.txt")]));
        assert_eq!(
            v["duplicates"],
            json!([{"id": "2a3b4c5d6e7f", "record_paths": [format!("{ROOT}a.md"), format!("{ROOT}b.md")]}])
        );
        assert_eq!(
            v["supersede_problems"],
            json!([{"record_path": format!("{ROOT}c.md"), "problem": "not-in-folder"}, {"record_path": format!("{ROOT}d.md"), "problem": "itself"}])
        );
        assert_eq!(v["cycles"], json!([["5d8e2c1b9a3f", "6e7f8g9h0j1k"]]));
        assert_eq!(v["records"][6]["record_path"], format!("{ROOT}g\u{1b}[2J.md"), "JSON is unescaped");
        assert_eq!(
            [thousands(0), thousands(999), thousands(1000), thousands(1_234_567)],
            ["0", "999", "1,000", "1,234,567"]
        );
    }

    #[test]
    fn review_unknown_is_said_once_after_the_summary() {
        let mut t = MemTree::new();
        t.file(&format!("{ROOT}a.md"), rec("2a3b4c5d6e7f", "fact", "A", ""));
        let text = render(&t, &Review::Unknown);
        assert!(text.ends_with(&format!("superseded.\n{REVIEW_UNKNOWN}\n")), "{text}");
        assert!(!render(&t, &Review::Known(BTreeSet::new())).contains(REVIEW_UNKNOWN));
        assert_eq!(render(&MemTree::new(), &Review::Unknown), "No records.\n");
    }

    #[test]
    fn pathspec_matches_paths_and_globs() {
        let r = Repo::new();
        std::fs::create_dir_all(r.dir().join("charts/api")).unwrap();
        let s = Session::open(&r.dir().join("charts")).unwrap();
        let spec = |args: &[&str]| Pathspec::parse(&s, &args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>());
        let all = spec(&[]).unwrap();
        assert!(all.matches("anything/at/all.yaml#k"));
        let api = spec(&["api/"]).unwrap();
        assert!(api.matches("charts/api/values.yaml#replicaCount") && api.matches("charts/api"));
        assert!(!api.matches("charts/apis/values.yaml") && !api.matches("charts/values.yaml"));
        let glob = spec(&["*/values.yaml"]).unwrap();
        assert!(glob.matches("charts/api/values.yaml#replicaCount") && !glob.matches("charts/api/x/values.yaml"));
        assert!(spec(&[".."]).unwrap().matches("web/a.ts"), "the top names every file");
        assert_eq!(
            spec(&[":(exclude)api"]).err().unwrap(),
            ":(exclude)api: pathspec magic is not supported; name a path or a glob"
        );
        assert_eq!(spec(&["../../x"]).err().unwrap(), "../../x is outside the repository");
        assert_eq!(spec(&["api", "../web"]).unwrap().typed(), "api, ../web");
        // A file whose name holds `#`, named exactly, is inside, alone or before a key path; never failing open.
        let hash = spec(&["a#b.md", "v#1.yaml"]).unwrap();
        assert!(hash.matches("charts/a#b.md") && hash.matches("charts/v#1.yaml#replicaCount"));
        assert!(!hash.matches("charts/a#c.md") && !hash.matches("charts/v#2.yaml#replicaCount"));
    }
}
