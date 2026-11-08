// SPDX-License-Identifier: MIT
//! `docbrain status --base`: the change gate (`SEMANTICS.md` §The gate) between B and H, as text, as JSON and as
//! an exit that fails only on a broken finding under the pathspec, and only when asked.

use std::collections::{BTreeMap, BTreeSet};

use docbrain_knowledge::folder::{self, scope_of};
use docbrain_knowledge::gate::{ChangeFinding, ChangeKind, Class, Finding, GateReport, KnowledgeChanges};
use docbrain_knowledge::record::{Kind, Parsed, parse_record};
use docbrain_knowledge::secret::screen;
use docbrain_knowledge::{Oid, Tree, TreeError};
use serde_json::{Value, json};

use super::args::StatusArgs;
use super::exit::Exit;
use super::frame::{Frame, NOTICE, emit, esc, quote};
use super::paths::Session;
use super::sarif::{Files, sarif};
use super::show::{WITHHELD_VALUE, anchor_path, anchored_files};
use super::status::{Pathspec, verdict};
use super::trees::{open_trees, tree_error_line};

/// The most findings the text shows; JSON lists every one. Knowledge changes are never capped (edge case 35).
const SHOWN: usize = 10;
/// The width of a finding's label.
const LABEL: usize = 7;
/// A finding's further lines start under its anchor.
const MORE: &str = "           ";
/// The one reason a finding keeps when its record holds a secret shape (`SEMANTICS.md` §Secrets).
pub(crate) const WITHHELD: &str = "withheld: secret-shape";

/// What the exit is made of: broken findings under the pathspec, and those outside it.
pub(crate) struct GateOutcome {
    pub fail_on: bool,
    pub failing: usize,
    pub outside: usize,
}

impl GateOutcome {
    fn exit(&self) -> Exit {
        if self.fail_on && self.failing > 0 { Exit::Fail } else { Exit::Ok }
    }
}

pub(crate) fn run_gate(s: &Session, args: &StatusArgs, base: &str, spec: &Pathspec) -> Result<Exit, String> {
    let j = open_trees(s, args.rev.as_deref(), Some(base))?;
    let (Some((b, base_id)), Some(changed)) = (&j.base, &j.changed) else {
        return Err(format!("no merge base with {base}"));
    };
    let mut report = docbrain_knowledge::gate::gate(b, &j.h, changed).map_err(|e| tree_error_line(&e))?;
    withhold_secrets(&mut report);
    let out = outcome(&report, args.fail_on.is_some(), spec);
    let text = if args.json {
        let head = match (&args.rev, &j.h_commit) {
            (Some(_), Some(c)) => c.as_str().to_owned(),
            _ => "working tree".to_owned(),
        };
        format!("{:#}\n", gate_json(&report, base_id, &head, &out))
    } else if args.sarif {
        let files = files_of(&report, &j.h, b).map_err(|e| tree_error_line(&e))?;
        format!("{:#}\n", sarif(&report, &files, env!("CARGO_PKG_VERSION")))
    } else {
        let frame = Frame::new().map_err(|e| format!("{e:#}"))?;
        let mut text = render_gate(&report, &kinds(&report, &j.h, b), &frame);
        if out.fail_on {
            text += &verdict(out.failing, out.outside, spec);
        }
        text
    };
    emit(&text).map_err(|e| format!("cannot write the output: {e}"))?;
    Ok(out.exit())
}

/// The broken findings whose anchored file is in the pathspec, and those that are not.
pub(crate) fn outcome(r: &GateReport, fail_on: bool, ps: &Pathspec) -> GateOutcome {
    let (mut failing, mut outside) = (0, 0);
    for f in r.findings.iter().filter(|f| matches!(f.class, Class::Broken { .. })) {
        if ps.matches(&f.anchor) {
            failing += 1;
        } else {
            outside += 1;
        }
    }
    GateOutcome { fail_on, failing, outside }
}

/// A finding keeps no text in any output when its record is one the change added or edited with a secret shape in
/// it, or when its own text holds one (a record from before the change, a value read for a reason): no title, no
/// first paragraph, one reason and no expectation or anchor in its class, so a log never holds the secret.
pub(crate) fn withhold_secrets(r: &mut GateReport) {
    let secret: BTreeSet<String> = r
        .change_findings
        .iter()
        .filter_map(|c| match c {
            ChangeFinding::SecretShape { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    let leaks = |f: &Finding| {
        secret.contains(&f.record_path)
            || [&f.title, &f.first_paragraph].into_iter().chain(&f.reasons).any(|t| !screen(t).is_empty())
    };
    for f in r.findings.iter_mut().filter(|f| leaks(f)) {
        f.title.clear();
        f.first_paragraph.clear();
        f.reasons = vec![WITHHELD.to_owned()];
        if let Class::Changed(ChangeKind::AnchorMoved { from, to } | ChangeKind::ExpectationChanged { from, to }) =
            &mut f.class
        {
            (*from, *to) = (WITHHELD_VALUE.to_owned(), WITHHELD_VALUE.to_owned());
        }
    }
}

/// The kind of each shown finding's record, read from H, else from B (a record the change removed or broke).
fn kinds(r: &GateReport, head: &dyn Tree, base: &dyn Tree) -> BTreeMap<String, Kind> {
    let mut out = BTreeMap::new();
    for f in r.findings.iter().take(SHOWN) {
        if !out.contains_key(&f.record_path)
            && let Some(k) = kind_at(head, &f.record_path).or_else(|| kind_at(base, &f.record_path))
        {
            out.insert(f.record_path.clone(), k);
        }
    }
    out
}

fn kind_at(tree: &dyn Tree, path: &str) -> Option<Kind> {
    match record_at(tree, path)? {
        Parsed::Valid(rec) => Some(rec.kind),
        Parsed::Newer { kind, .. } => kind,
        Parsed::Invalid { .. } => None,
    }
}

fn record_at(tree: &dyn Tree, path: &str) -> Option<Parsed> {
    let scope = scope_of(path)?;
    let entry = tree.entry(path).ok()??;
    let bytes = tree.read_many(std::slice::from_ref(&entry.oid)).ok()?.into_iter().next()?;
    Some(parse_record(scope, &bytes))
}

/// Each finding's anchored file, from the check of its record (at H, else at B) whose anchor it names; and, when the
/// change has a cycle, the record file of every id at H.
fn files_of(r: &GateReport, head: &dyn Tree, base: &dyn Tree) -> Result<Files, TreeError> {
    let mut files = Files::default();
    for f in &r.findings {
        let key = (f.record_path.clone(), f.anchor.clone());
        if files.anchored.contains_key(&key) {
            continue;
        }
        let trees: [&dyn Tree; 2] = [head, base];
        let found = trees.into_iter().find_map(|t| match record_at(t, &f.record_path)? {
            Parsed::Valid(rec) => rec
                .checks
                .iter()
                .find(|c| anchor_path(c) == f.anchor)
                .and_then(|c| anchored_files(c).first().map(|file| (*file).to_owned())),
            _ => None,
        });
        if let Some(file) = found {
            files.anchored.insert(key, file);
        }
    }
    if r.change_findings.iter().any(|c| matches!(c, ChangeFinding::Cycle { .. })) {
        for rs in folder::status(head)?.records {
            if let Some(id) = rs.id {
                files.of_id.entry(id.as_str().to_owned()).or_insert(rs.path);
            }
        }
    }
    Ok(files)
}

/// The text of the gate: the header, at most ten findings, every knowledge change and every finding on the change,
/// the frame's close and the summary; the summary alone when there is nothing to quote.
pub(crate) fn render_gate(r: &GateReport, kinds: &BTreeMap<String, Kind>, frame: &Frame) -> String {
    let k = &r.knowledge;
    let proposed: BTreeSet<&str> = k.added.iter().chain(&k.edited).map(String::as_str).collect();
    let mut inner = String::new();
    for f in r.findings.iter().take(SHOWN) {
        inner += &finding_block(f, kinds.get(&f.record_path).copied(), proposed.contains(f.record_path.as_str()));
    }
    let more = r.findings.len().saturating_sub(SHOWN);
    if more > 0 {
        let findings = if more == 1 { "finding" } else { "findings" };
        inner += &format!("  {more} more {findings}: --json or --sarif lists every one.\n");
    }
    inner += &knowledge_lines(k);
    inner += &change_lines(&r.change_findings);
    if inner.is_empty() {
        return summary(r);
    }
    frame.header() + &inner + &frame.close() + &summary(r)
}

fn finding_block(f: &Finding, kind: Option<Kind>, proposed: bool) -> String {
    let (word, rule) = match &f.class {
        Class::Broken { rule } => ("broken", format!(" (rule {rule})")),
        Class::Changed(_) => ("changed", String::new()),
        Class::Stale => ("stale", String::new()),
    };
    let mut out = format!("  {word:<LABEL$}  {}{rule}\n", esc(&f.anchor));
    let mut more = |line: &str| {
        out.push_str(MORE);
        out.push_str(line);
        out.push('\n');
    };
    if !f.title.is_empty() {
        match kind {
            Some(k) => more(&format!("{k}: {}", esc(&f.title))),
            None => more(&esc(&f.title)),
        }
    }
    let note = if proposed { " (not reviewed: proposed in this branch)" } else { "" };
    more(&format!("{}{note}", esc(&f.record_path)));
    for reason in &f.reasons {
        more(&format!("reason: {}", esc(reason)));
    }
    if let Some(p) = &f.same_content {
        more(&format!("same content at {}", esc(p)));
    }
    out + &quote(&f.first_paragraph, MORE.len())
}

fn knowledge_lines(k: &KnowledgeChanges) -> String {
    let mut out = String::new();
    let mut line = |label: &str, rest: &str| out += &format!("  knowledge  {label:<8}  {rest}\n");
    for p in &k.added {
        line("added", &esc(p));
    }
    for p in &k.edited {
        line("edited", &esc(p));
    }
    for p in &k.removed {
        line("removed", &esc(p));
    }
    for (from, to) in &k.followed {
        line("followed", &format!("{} -> {}", esc(from), esc(to)));
    }
    for (p, i, code) in &k.new_unverifiable {
        line("not judged", &format!("{} check {}: {}", esc(p), i + 1, esc(code)));
    }
    out
}

fn change_lines(c: &[ChangeFinding]) -> String {
    c.iter().map(|c| format!("  on the change  {}\n", change_text(c))).collect()
}

/// A finding on the change in one line, as text and SARIF both say it: its kind, its files, never a secret's text.
pub(crate) fn change_text(c: &ChangeFinding) -> String {
    let paths = |ps: &[String]| ps.iter().map(|p| esc(p)).collect::<Vec<_>>().join(", ");
    match c {
        ChangeFinding::DuplicateId { id, paths: ps } => format!("duplicate-id {}: {}", id.as_str(), paths(ps)),
        ChangeFinding::Supersedes { path, problem } => {
            format!("supersedes {}: {}", esc(path), json!(problem).as_str().unwrap_or_default())
        }
        ChangeFinding::Cycle { ids } => {
            format!("cycle {}", ids.iter().map(|id| id.as_str()).collect::<Vec<_>>().join(", "))
        }
        ChangeFinding::SecretShape { path, rule, line, col } => {
            format!("secret-shape {} {line}:{col} ({rule})", esc(path))
        }
    }
}

fn summary(r: &GateReport) -> String {
    let n = |f: fn(&Class) -> bool| r.findings.iter().filter(|x| f(&x.class)).count();
    let mut s = format!(
        "{} broken, {} changed, {} stale.",
        n(|c| matches!(c, Class::Broken { .. })),
        n(|c| matches!(c, Class::Changed(_))),
        n(|c| matches!(c, Class::Stale)),
    );
    if r.broken_before > 0 {
        s += &format!(" {} broken before this change, not counted.", r.broken_before);
    }
    s + "\n"
}

/// `status --base --json`: the crate's report as it is (a secret-shaped record's text withheld), the two trees,
/// and what the exit is made of.
pub(crate) fn gate_json(r: &GateReport, base: &Oid, head: &str, gate: &GateOutcome) -> Value {
    json!({
        "format": 1,
        "notice": NOTICE,
        "base": base.as_str(),
        "head": head,
        "report": r,
        "gate": {"fail_on": gate.fail_on, "failing": gate.failing, "outside_pathspec": gate.outside},
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::testrepo::Repo;
    use docbrain_knowledge::record::Id;

    fn finding(class: Class, path: &str, anchor: &str) -> Finding {
        Finding {
            class,
            id: Id::parse("7k2m9q4r8t1v").unwrap(),
            record_path: path.to_owned(),
            anchor: anchor.to_owned(),
            title: "Title".to_owned(),
            first_paragraph: "Why.".to_owned(),
            reasons: vec!["missing-file".to_owned()],
            same_content: None,
        }
    }

    fn render(r: &GateReport) -> String {
        render_gate(r, &BTreeMap::new(), &Frame::fixed("T"))
    }

    #[test]
    fn ten_findings_are_shown_and_every_knowledge_change() {
        let more = |n: usize| match n {
            0 => None,
            1 => Some("  1 more finding: --json or --sarif lists every one.".to_owned()),
            n => Some(format!("  {n} more findings: --json or --sarif lists every one.")),
        };
        for n in [9, 10, 11, 25] {
            let r = GateReport {
                findings: (0..n).map(|i| finding(Class::Broken { rule: 1 }, &format!("r{i:02}.md"), "a")).collect(),
                knowledge: KnowledgeChanges {
                    added: (0..40).map(|i| format!("k{i:02}.md")).collect(),
                    ..Default::default()
                },
                ..Default::default()
            };
            let text = render(&r);
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(lines.iter().filter(|l| l.starts_with("  broken ")).count(), n.min(10), "{text}");
            assert_eq!(lines.iter().filter(|l| l.starts_with("  knowledge  added     ")).count(), 40, "{text}");
            let first_knowledge = lines.iter().position(|l| l.starts_with("  knowledge")).unwrap();
            let before = lines[first_knowledge - 1];
            match more(n.saturating_sub(10)) {
                Some(m) => assert_eq!(before, m),
                None => assert_eq!(before, "           > Why."),
            }
            assert!(text.ends_with(&format!("--- end T\n{n} broken, 0 changed, 0 stale.\n")), "{text}");
        }
    }

    #[test]
    fn a_secret_shaped_record_keeps_no_text() {
        let mut r = GateReport {
            findings: vec![finding(Class::Broken { rule: 1 }, "a.md", "x"), finding(Class::Stale, "b.md", "y")],
            change_findings: vec![ChangeFinding::SecretShape {
                path: "a.md".to_owned(),
                rule: "aws_access_key_id",
                line: 8,
                col: 19,
            }],
            ..Default::default()
        };
        withhold_secrets(&mut r);
        let a = &r.findings[0];
        assert_eq!((a.title.as_str(), a.first_paragraph.as_str()), ("", ""));
        assert_eq!(a.reasons, [WITHHELD]);
        assert_eq!(r.findings[1], finding(Class::Stale, "b.md", "y"), "another record keeps its text");
        let text = render(&r);
        let block = "  broken   x (rule 1)\n           a.md\n           reason: withheld: secret-shape\n  stale    y\n";
        assert!(text.contains(block), "{text}");
        assert!(text.contains("\n  on the change  secret-shape a.md 8:19 (aws_access_key_id)\n"), "{text}");
    }

    #[test]
    fn text_holding_a_secret_is_withheld_whoever_wrote_it() {
        let key = "AKIAQ7ZK3M9W2X4V8B6N";
        let mut paragraph = finding(Class::Broken { rule: 1 }, "a.md", "x");
        paragraph.first_paragraph = format!("Rotate {key} yearly.");
        let mut reason = finding(Class::Broken { rule: 1 }, "b.md", "y");
        reason.reasons = vec![format!("false: {key}, expected == x")];
        let mut title = finding(Class::Stale, "c.md", "z");
        title.title = format!("Key {key}");
        let changed = ChangeKind::ExpectationChanged { from: ">= 2".to_owned(), to: format!("== {key}") };
        let mut class = finding(Class::Changed(changed), "e.md", "v");
        class.reasons = vec![format!("expectation-changed: >= 2 -> == {key}")];
        let mut documented = finding(Class::Stale, "d.md", "w");
        documented.first_paragraph = "postgres://app:changeme@db/app".to_owned();
        let mut r =
            GateReport { findings: vec![paragraph, reason, title, class, documented.clone()], ..Default::default() };
        withhold_secrets(&mut r);
        for f in &r.findings[..4] {
            assert_eq!((f.title.as_str(), f.first_paragraph.as_str()), ("", ""), "{f:?}");
            assert_eq!(f.reasons, [WITHHELD], "{f:?}");
        }
        let withheld =
            ChangeKind::ExpectationChanged { from: WITHHELD_VALUE.to_owned(), to: WITHHELD_VALUE.to_owned() };
        assert_eq!(r.findings[3].class, Class::Changed(withheld), "the class carries the expectation too");
        assert_eq!(r.findings[4], documented, "a documented shape is text like any other");
        assert!(!serde_json::to_string(&r).unwrap().contains(key));
    }

    #[test]
    fn outcome_counts_broken_findings_by_the_pathspec() {
        let repo = Repo::new();
        std::fs::create_dir_all(repo.dir().join("apps")).unwrap();
        let s = Session::open(repo.dir()).unwrap();
        let ps = |a: &[&str]| Pathspec::parse(&s, &a.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>()).unwrap();
        let r = GateReport {
            findings: vec![
                finding(Class::Broken { rule: 1 }, "a.md", "apps/x.yaml#replicas"),
                finding(Class::Broken { rule: 2 }, "b.md", "charts/v.yaml"),
                finding(Class::Changed(ChangeKind::Superseded), "c.md", "apps/y.yaml"),
                finding(Class::Stale, "d.md", "apps/z.yaml"),
            ],
            ..Default::default()
        };
        let o = outcome(&r, true, &ps(&["apps/"]));
        assert_eq!((o.failing, o.outside, o.exit()), (1, 1, Exit::Fail));
        let o = outcome(&r, false, &ps(&["apps/"]));
        assert_eq!((o.failing, o.outside, o.exit()), (1, 1, Exit::Ok), "without --fail-on nothing fails");
        let o = outcome(&r, true, &ps(&["docs/"]));
        assert_eq!((o.failing, o.outside, o.exit()), (0, 2, Exit::Ok), "changed and stale never count");
        let o = outcome(&r, true, &ps(&[]));
        assert_eq!((o.failing, o.outside), (2, 0));
    }

    #[test]
    fn nothing_to_quote_is_the_summary_alone() {
        let mut r = GateReport::default();
        assert_eq!(render(&r), "0 broken, 0 changed, 0 stale.\n");
        r.broken_before = 2;
        assert_eq!(render(&r), "0 broken, 0 changed, 0 stale. 2 broken before this change, not counted.\n");
        r.knowledge.followed = vec![("a\u{1b}.yaml".to_owned(), "b.yaml".to_owned())];
        assert_eq!(
            render(&r),
            format!(
                "{}  knowledge  followed  a\\x{{1b}}.yaml -> b.yaml\n--- end T\n0 broken, 0 changed, 0 stale. 2 broken before this change, not counted.\n",
                Frame::fixed("T").header()
            )
        );
    }
}
