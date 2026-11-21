// SPDX-License-Identifier: MIT
//! `docbrain status --base`: the change gate (`SEMANTICS.md` §The gate) between B and H, as text, as JSON and as
//! an exit that fails only on a broken finding under the pathspec, and only when asked.

use std::collections::BTreeSet;

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
use super::show::{WITHHELD_VALUE, anchor_path, anchored_files, shown_anchor};
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
/// The one action for a broken finding, said once under the summary, in SARIF's help for the rule, and in the
/// folder README: a constraint comes down in a change of its own (`SEMANTICS.md` §What the two rules mean in
/// practice).
pub(crate) const ON_PURPOSE: &str = "To change a guarded value on purpose, relax, supersede or remove its record in a \
                                     change of its own; once that is merged, change the value.";

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
    // The pathspec judges each anchor as written: one withheld below still lies where it lay.
    let out = outcome(&report, args.fail_on.is_some(), spec);
    withhold_secrets(&mut report);
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

/// No output holds text read from a record file the change added or edited with a secret shape in it, so a log never
/// holds the secret. Each part of a finding is withheld on its own, by its own screen and by the record it was read
/// from (`SEMANTICS.md` §Output): the title and first paragraph are the judging record's, H's only under rule 1 and
/// for `stale`; each reason, and the new expectation or anchor of a `changed` class, holds text of the H record
/// `Finding::read_at_head` names, if any; the old one is B's. A withheld title or paragraph is empty, a withheld
/// reason reads `withheld: secret-shape` (two in a row read once), a withheld expectation or anchor
/// `(withheld: secret-shape)`. Text that holds a secret shape itself (a record from before the change, a value read
/// for a reason) is withheld whoever wrote it; a value read for a reason reaches here already withheld, screened whole
/// before its cut (`Shown`). An anchor, a finding's, a moved one or a followed one, is shown without its
/// secret-shaped part (`shown_anchor`).
pub(crate) fn withhold_secrets(r: &mut GateReport) {
    let secret: BTreeSet<String> = r
        .change_findings
        .iter()
        .filter_map(|c| match c {
            ChangeFinding::SecretShape { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    let read_at_secret = |p: Option<&String>| p.is_some_and(|p| secret.contains(p));
    for f in &mut r.findings {
        let head = judged_at_head(&f.class) && secret.contains(&f.record_path);
        for text in [&mut f.title, &mut f.first_paragraph] {
            if head || !screen(text).is_empty() {
                text.clear();
            }
        }
        let mut reasons: Vec<String> = Vec::with_capacity(f.reasons.len());
        for (i, reason) in std::mem::take(&mut f.reasons).into_iter().enumerate() {
            if !read_at_secret(f.read_at_head.reasons.get(i).and_then(Option::as_ref)) && screen(&reason).is_empty() {
                reasons.push(reason);
            } else if reasons.last().map(String::as_str) != Some(WITHHELD) {
                reasons.push(WITHHELD.to_owned());
            }
        }
        f.reasons = reasons;
        let new_at_secret = read_at_secret(f.read_at_head.class.as_ref());
        match &mut f.class {
            Class::Changed(ChangeKind::AnchorMoved { from, to }) => {
                *from = shown_anchor(from).into_owned();
                *to = if new_at_secret { WITHHELD_VALUE.to_owned() } else { shown_anchor(to).into_owned() };
            }
            Class::Changed(ChangeKind::ExpectationChanged { from, to }) => {
                for (value, withheld) in [(from, false), (to, new_at_secret)] {
                    if withheld || !screen(value).is_empty() {
                        *value = WITHHELD_VALUE.to_owned();
                    }
                }
            }
            _ => {}
        }
        f.anchor = shown_anchor(&f.anchor).into_owned();
    }
    // Two pairs that read the same once withheld are one line.
    let mut shown = BTreeSet::new();
    r.knowledge.followed = std::mem::take(&mut r.knowledge.followed)
        .into_iter()
        .map(|(from, to)| (shown_anchor(&from).into_owned(), shown_anchor(&to).into_owned()))
        .filter(|pair| shown.insert(pair.clone()))
        .collect();
}

/// Whether H's record judged a finding, so its title and paragraph are H's text: rule 1 and `stale`. Rule 2 and
/// `changed` quote B's record, the reviewed one (`SEMANTICS.md` §Output).
fn judged_at_head(c: &Class) -> bool {
    matches!(c, Class::Broken { rule: 1 } | Class::Stale)
}

/// The kind of each shown finding's record, read from the tree whose record judged it, as its title is.
fn kinds(r: &GateReport, head: &dyn Tree, base: &dyn Tree) -> Vec<Option<Kind>> {
    let tree = |f: &Finding| if judged_at_head(&f.class) { head } else { base };
    r.findings.iter().take(SHOWN).map(|f| kind_at(tree(f), &f.record_path)).collect()
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
/// the frame's close and the summary; the summary alone when there is nothing to quote. `kinds` holds the kind of
/// each shown finding's record, in order.
pub(crate) fn render_gate(r: &GateReport, kinds: &[Option<Kind>], frame: &Frame) -> String {
    let k = &r.knowledge;
    let proposed: BTreeSet<&str> = k.added.iter().chain(&k.edited).map(String::as_str).collect();
    let mut inner = String::new();
    for (i, f) in r.findings.iter().take(SHOWN).enumerate() {
        // Only H's text is proposed; a finding quoting B's record is unlabelled, the path still listed as edited.
        let unreviewed = judged_at_head(&f.class) && proposed.contains(f.record_path.as_str());
        inner += &finding_block(f, kinds.get(i).copied().flatten(), unreviewed);
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

/// The counts, then the one action when a broken finding is shown.
fn summary(r: &GateReport) -> String {
    let n = |f: fn(&Class) -> bool| r.findings.iter().filter(|x| f(&x.class)).count();
    let broken = n(|c| matches!(c, Class::Broken { .. }));
    let mut s = format!(
        "{broken} broken, {} changed, {} stale.",
        n(|c| matches!(c, Class::Changed(_))),
        n(|c| matches!(c, Class::Stale)),
    );
    if r.broken_before > 0 {
        s += &format!(" {} broken before this change, not counted.", r.broken_before);
    }
    if broken > 0 {
        s += &format!("\n{ON_PURPOSE}");
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
    use docbrain_knowledge::gate::ReadAtHead;
    use docbrain_knowledge::record::Id;

    fn finding(class: Class, path: &str, anchor: &str) -> Finding {
        // As the evaluator reads it: rule 1's and a stale note's reason is the text of H's record, the others B's.
        let at_head = matches!(class, Class::Broken { rule: 1 } | Class::Stale).then(|| path.to_owned());
        Finding {
            class,
            id: Id::parse("7k2m9q4r8t1v").unwrap(),
            record_path: path.to_owned(),
            anchor: anchor.to_owned(),
            title: "Title".to_owned(),
            first_paragraph: "Why.".to_owned(),
            reasons: vec!["missing-file".to_owned()],
            same_content: None,
            read_at_head: ReadAtHead { reasons: vec![at_head], class: None },
        }
    }

    /// A finding's title, first paragraph and reasons.
    fn parts(f: &Finding) -> (&str, &str, Vec<&str>) {
        (&f.title, &f.first_paragraph, f.reasons.iter().map(String::as_str).collect())
    }

    fn render(r: &GateReport) -> String {
        render_gate(r, &[], &Frame::fixed("T"))
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
            assert!(text.ends_with(&format!("--- end T\n{n} broken, 0 changed, 0 stale.\n{ON_PURPOSE}\n")), "{text}");
        }
    }

    /// Only text quoted from H's record is proposed: rule 1 and `stale` quote it, rule 2 and `changed` quote B's
    /// reviewed copy (`SEMANTICS.md` §Output), whatever the change did to the record's path.
    #[test]
    fn the_label_follows_the_record_that_judged_the_finding() {
        let expectation = ChangeKind::ExpectationChanged { from: ">= 2".to_owned(), to: ">= 1".to_owned() };
        let r = GateReport {
            findings: vec![
                finding(Class::Broken { rule: 1 }, "added-1.md", "a"),
                finding(Class::Broken { rule: 1 }, "edited-1.md", "b"),
                finding(Class::Broken { rule: 1 }, "kept-1.md", "c"),
                finding(Class::Broken { rule: 2 }, "edited-2.md", "d"),
                finding(Class::Changed(expectation), "edited-changed.md", "e"),
                finding(Class::Changed(ChangeKind::RecordRemoved), "removed.md", "f"),
                finding(Class::Stale, "edited-stale.md", "g"),
                finding(Class::Stale, "kept-stale.md", "h"),
            ],
            knowledge: KnowledgeChanges {
                added: vec!["added-1.md".to_owned()],
                edited: ["edited-1.md", "edited-2.md", "edited-changed.md", "edited-stale.md"]
                    .map(String::from)
                    .to_vec(),
                removed: vec!["removed.md".to_owned()],
                ..Default::default()
            },
            ..Default::default()
        };
        let text = render(&r);
        let path_line =
            |p: &str| text.lines().find(|l| l.trim_start().starts_with(p)).unwrap_or_else(|| panic!("{p}: {text}"));
        let label = " (not reviewed: proposed in this branch)";
        for p in ["added-1.md", "edited-1.md", "edited-stale.md"] {
            assert_eq!(path_line(p), format!("{MORE}{p}{label}"), "H's record, added or edited: {text}");
        }
        for p in ["kept-1.md", "edited-2.md", "edited-changed.md", "removed.md", "kept-stale.md"] {
            assert_eq!(path_line(p), format!("{MORE}{p}"), "B's record, or a path the change kept: {text}");
        }
        for p in ["edited-2.md", "edited-changed.md"] {
            assert!(text.contains(&format!("\n  knowledge  edited    {p}\n")), "still listed as edited: {text}");
        }
    }

    #[test]
    fn a_broken_finding_ends_with_how_to_change_the_value_on_purpose() {
        for rule in [1, 2] {
            let findings =
                vec![finding(Class::Broken { rule }, "a.md", "x"), finding(Class::Broken { rule }, "b.md", "y")];
            let text = render(&GateReport { findings, ..Default::default() });
            assert!(text.ends_with(&format!("--- end T\n2 broken, 0 changed, 0 stale.\n{ON_PURPOSE}\n")), "{text}");
            assert_eq!(text.matches(ON_PURPOSE).count(), 1, "said once: {text}");
        }
        for class in [Class::Changed(ChangeKind::RecordRemoved), Class::Stale] {
            let r = GateReport { findings: vec![finding(class, "a.md", "x")], broken_before: 1, ..Default::default() };
            let text = render(&r);
            assert!(!text.contains(ON_PURPOSE), "no broken finding shown: {text}");
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
    fn only_text_read_from_a_secret_shaped_record_is_withheld() {
        // a.md, at H, holds a secret shape; c.md does not. Each part is withheld by the record it was read from.
        let key = "AKIAQ7ZK3M9W2X4V8B6N";
        let reading = |class: Class, path: &str, reasons: &[(&str, Option<&str>)], new: Option<&str>| Finding {
            reasons: reasons.iter().map(|(t, _)| (*t).to_owned()).collect(),
            read_at_head: ReadAtHead {
                reasons: reasons.iter().map(|(_, p)| p.map(str::to_owned)).collect(),
                class: new.map(str::to_owned),
            },
            ..finding(class, path, "v.yaml#replicas")
        };
        let (b2, b3, relaxed_reason) = ("false: 1, expected >= 2", "false: 1, expected >= 3", "expectation-changed: >= 2 -> >= 1");
        let relaxed = || Class::Changed(ChangeKind::ExpectationChanged { from: ">= 2".to_owned(), to: ">= 1".to_owned() });
        let moved = ChangeKind::AnchorMoved { from: "keys/current.yaml#owner".to_owned(), to: format!("keys/{key}.yaml#owner") };
        let moved_reason = format!("anchor-moved: keys/current.yaml#owner -> keys/{key}.yaml#owner");
        let clean_move = || ChangeKind::AnchorMoved { from: "v.yaml#replicas".to_owned(), to: "w.yaml#replicas".to_owned() };
        let a = Some("a.md");
        let mut r = GateReport {
            findings: vec![
                reading(Class::Broken { rule: 2 }, "a.md", &[(b2, None)], None),
                reading(Class::Broken { rule: 2 }, "a.md", &[(b2, None), (b3, a)], None),
                reading(Class::Broken { rule: 2 }, "a.md", &[(b3, a), (b2, None), (b3, a), (b3, a)], None),
                reading(relaxed(), "a.md", &[(relaxed_reason, a)], a),
                reading(relaxed(), "b.md", &[(relaxed_reason, a)], a),
                reading(Class::Changed(moved), "a.md", &[(moved_reason.as_str(), a)], a),
                reading(Class::Broken { rule: 1 }, "c.md", &[(b2, Some("c.md"))], None),
                reading(Class::Stale, "a.md", &[(b3, a)], None),
                reading(Class::Changed(clean_move()), "a.md", &[("anchor-moved: v.yaml#replicas -> w.yaml#replicas", a)], a),
            ],
            change_findings: vec![ChangeFinding::SecretShape {
                path: "a.md".to_owned(),
                rule: "aws_access_key_id",
                line: 8,
                col: 19,
            }],
            ..Default::default()
        };
        let before = r.findings.clone();
        withhold_secrets(&mut r);
        let got: Vec<_> = r.findings.iter().map(parts).collect();
        assert_eq!(
            got,
            [
                ("Title", "Why.", vec![b2]),
                ("Title", "Why.", vec![b2, WITHHELD]),
                ("Title", "Why.", vec![WITHHELD, b2, WITHHELD]),
                ("Title", "Why.", vec![WITHHELD]),
                ("Title", "Why.", vec![WITHHELD]),
                ("Title", "Why.", vec![WITHHELD]),
                ("Title", "Why.", vec![b2]),
                ("", "", vec![WITHHELD]),
                ("Title", "Why.", vec![WITHHELD]),
            ]
        );
        let relaxed = Class::Changed(ChangeKind::ExpectationChanged { from: ">= 2".to_owned(), to: WITHHELD_VALUE.to_owned() });
        assert_eq!((&r.findings[3].class, &r.findings[4].class), (&relaxed, &relaxed), "B's expectation, H's withheld");
        let moved = ChangeKind::AnchorMoved { from: "keys/current.yaml#owner".to_owned(), to: WITHHELD_VALUE.to_owned() };
        assert_eq!(r.findings[5].class, Class::Changed(moved), "B's anchor, H's withheld");
        let moved = ChangeKind::AnchorMoved { from: "v.yaml#replicas".to_owned(), to: WITHHELD_VALUE.to_owned() };
        assert_eq!(r.findings[8].class, Class::Changed(moved), "H's anchor is withheld with its record, shape or not");
        assert_eq!((&r.findings[0], &r.findings[6]), (&before[0], &before[6]));
        assert!(!serde_json::to_string(&r).unwrap().contains(key));
    }

    #[test]
    fn a_part_holding_a_secret_is_withheld_whoever_wrote_it() {
        let key = "AKIAQ7ZK3M9W2X4V8B6N";
        let mut paragraph = finding(Class::Broken { rule: 1 }, "a.md", "x");
        paragraph.first_paragraph = format!("Rotate {key} yearly.");
        let mut reason = finding(Class::Broken { rule: 1 }, "b.md", "y");
        reason.reasons = vec![format!("false: {key}, expected == x")];
        let mut title = finding(Class::Stale, "c.md", "z");
        title.title = format!("Key {key}");
        let changed = ChangeKind::ExpectationChanged { from: ">= 2".to_owned(), to: format!("== {key}") };
        let mut new = finding(Class::Changed(changed), "e.md", "v");
        new.reasons = vec![format!("expectation-changed: >= 2 -> == {key}")];
        let changed = ChangeKind::ExpectationChanged { from: format!("== {key}"), to: ">= 2".to_owned() };
        let mut old = finding(Class::Changed(changed), "f.md", "u");
        old.reasons = vec![format!("expectation-changed: == {key} -> >= 2")];
        let mut documented = finding(Class::Stale, "d.md", "w");
        documented.first_paragraph = "postgres://app:changeme@db/app".to_owned();
        let findings = vec![paragraph, reason, title, new, old, documented.clone()];
        let mut r = GateReport { findings, ..Default::default() };
        withhold_secrets(&mut r);
        let got: Vec<_> = r.findings[..5].iter().map(parts).collect();
        assert_eq!(
            got,
            [
                ("Title", "", vec!["missing-file"]),
                ("Title", "Why.", vec![WITHHELD]),
                ("", "Why.", vec!["missing-file"]),
                ("Title", "Why.", vec![WITHHELD]),
                ("Title", "Why.", vec![WITHHELD]),
            ]
        );
        let new = ChangeKind::ExpectationChanged { from: ">= 2".to_owned(), to: WITHHELD_VALUE.to_owned() };
        let old = ChangeKind::ExpectationChanged { from: WITHHELD_VALUE.to_owned(), to: ">= 2".to_owned() };
        assert_eq!((&r.findings[3].class, &r.findings[4].class), (&Class::Changed(new), &Class::Changed(old)));
        assert_eq!(r.findings[5], documented, "a documented shape is text like any other");
        assert!(!serde_json::to_string(&r).unwrap().contains(key));
    }

    #[test]
    fn an_anchor_is_shown_without_its_secret_shaped_part() {
        let key = "AKIAQ7ZK3M9W2X4V8B6N";
        let moved = ChangeKind::AnchorMoved { from: format!("keys/{key}.yaml#a"), to: format!("v.yaml#tokens.{key}") };
        let mut changed = finding(Class::Changed(moved), "c.md", &format!("keys/{key}.yaml#a"));
        changed.reasons = vec![format!("anchor-moved: keys/{key}.yaml#a -> v.yaml#tokens.{key}")];
        let mut r = GateReport {
            findings: vec![
                finding(Class::Broken { rule: 1 }, "a.md", &format!("v.yaml#tokens.{key}")),
                finding(Class::Broken { rule: 2 }, "b.md", &format!("keys/{key}")),
                finding(Class::Stale, "d.md", "v.yaml#replicas"),
                changed,
            ],
            ..Default::default()
        };
        r.knowledge.followed = vec![
            (format!("keys/{key}.yaml"), "keys/current.yaml".to_owned()),
            ("keys/old.yaml".to_owned(), format!("keys/{key}.yaml")),
            (format!("keys/{key}.yml"), "keys/current.yaml".to_owned()),
        ];
        let before = r.findings[2].clone();
        withhold_secrets(&mut r);
        let anchors: Vec<&str> = r.findings.iter().map(|f| f.anchor.as_str()).collect();
        assert_eq!(anchors, ["v.yaml#(withheld: secret-shape)", WITHHELD_VALUE, "v.yaml#replicas", WITHHELD_VALUE]);
        // The anchor alone holds the shape: the rest of the finding is shown.
        assert_eq!((r.findings[0].title.as_str(), r.findings[0].reasons.as_slice()), ("Title", ["missing-file".to_owned()].as_slice()));
        assert_eq!(r.findings[2], before);
        // A moved anchor is shown as any anchor; the reason quoting it is withheld, not the title.
        let moved = ChangeKind::AnchorMoved { from: WITHHELD_VALUE.to_owned(), to: "v.yaml#(withheld: secret-shape)".to_owned() };
        assert_eq!((&r.findings[3].class, r.findings[3].reasons.as_slice()), (&Class::Changed(moved), [WITHHELD.to_owned()].as_slice()));
        assert_eq!(r.findings[3].title, "Title");
        // Two pairs that read the same once withheld are one line.
        let followed = [
            (WITHHELD_VALUE.to_owned(), "keys/current.yaml".to_owned()),
            ("keys/old.yaml".to_owned(), WITHHELD_VALUE.to_owned()),
        ];
        assert_eq!(r.knowledge.followed, followed);
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
