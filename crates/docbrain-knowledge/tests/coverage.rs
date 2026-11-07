// SPDX-License-Identifier: MIT
//! `conformance/coverage.tsv` names, for rules of `SEMANTICS.md` and for each of its edge cases, what proves it:
//! corpus cases, a test of this crate for a bound too large for a case, or the owner outside this crate (format
//! in `conformance/README.md`). This test keeps the map honest: every line names something that exists, every
//! edge case has its line, and that line states the edge case in the words of `SEMANTICS.md`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The parts of DocBrain outside this crate that carry a rule the evaluator does not decide.
const OWNERS: [&str; 4] = ["cli", "server", "job", "skill"];
/// The sections of `SEMANTICS.md` the map keys rules by; each has at least one line.
const SECTIONS: [&str; 12] = [
    "Reasons",
    "Bounds",
    "Paths and globs",
    "Hidden text",
    "The record",
    "Values and comparison",
    "Resolving values",
    "States",
    "Routing",
    "The gate",
    "Cost",
    "Secrets",
];
/// Lines keyed `Surfaces: <label>` are duties of the tools that carry records to people and agents, which
/// `SEMANTICS.md` does not state: they name owners, and tests of this crate where its git adapter does part of
/// the work, never corpus cases. A line keyed by a section is a rule of the evaluator: a case or a test proves it.
const SURFACES: &str = "Surfaces";
/// `SEMANTICS.md` §Edge cases lists 1 to this.
const EDGE_CASES: u32 = 87;

fn cases(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let p = e.unwrap().path();
        if p.is_dir() {
            cases(&p, out);
        } else if p.extension().is_some_and(|x| x == "json") {
            out.push(p);
        }
    }
}

/// `SEMANTICS.md` §Edge cases: each item's number and text, its continuation lines joined by one space.
fn edge_cases(semantics: &str) -> BTreeMap<u32, String> {
    let section = semantics.split("\n## Edge cases\n").nth(1).and_then(|s| s.split("\n## ").next()).expect("SEMANTICS.md has §Edge cases");
    let (mut items, mut last) = (BTreeMap::new(), None);
    for l in section.lines() {
        if let Some((n, text)) = l.split_once(". ").and_then(|(n, t)| n.parse::<u32>().ok().map(|n| (n, t))) {
            assert!(items.insert(n, text.to_owned()).is_none(), "edge case {n} is listed twice");
            last = Some(n);
        } else if let (Some(n), true) = (last, l.starts_with(' ') && !l.trim().is_empty()) {
            let text: &mut String = items.get_mut(&n).unwrap();
            text.push(' ');
            text.push_str(l.trim());
        } else {
            last = None;
        }
    }
    items
}

/// A token is a case id, `git` (git's own behaviour: no reader acts), an owner, or `test:<file>::<name>` for a
/// `#[test]` function of this crate.
fn token(t: &str, ids: &HashSet<String>, root: &Path) -> Result<(), String> {
    if t == "git" || ids.contains(t) || OWNERS.contains(&t) {
        return Ok(());
    }
    if let Some(rest) = t.strip_prefix("test:") {
        let (file, name) = rest.split_once("::").ok_or_else(|| format!("{t}: want test:<file>::<name>"))?;
        let src = std::fs::read_to_string(root.join(file)).map_err(|e| format!("{t}: {e}"))?;
        let lines: Vec<&str> = src.lines().collect();
        let at = lines.iter().position(|l| l.trim_start().starts_with(&format!("fn {name}("))).ok_or_else(|| format!("{t}: no fn {name}"))?;
        return if at > 0 && lines[at - 1].trim() == "#[test]" { Ok(()) } else { Err(format!("{t}: fn {name} is not a #[test]")) };
    }
    Err(format!("{t}: not a case id, an owner or a test"))
}

#[test]
fn every_rule_names_its_proof() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let semantics = std::fs::read_to_string(root.join("SEMANTICS.md")).expect("SEMANTICS.md");
    let headings: HashSet<&str> = semantics.lines().filter_map(|l| l.strip_prefix("## ")).collect();
    let edges = edge_cases(&semantics);
    assert_eq!(edges.keys().copied().collect::<Vec<_>>(), (1..=EDGE_CASES).collect::<Vec<_>>(), "SEMANTICS.md §Edge cases lists 1 to {EDGE_CASES}");
    for s in SECTIONS {
        assert!(headings.contains(s), "SEMANTICS.md has no section {s}");
    }
    let mut files = Vec::new();
    cases(&root.join("conformance/cases"), &mut files);
    let ids: HashSet<String> = files
        .iter()
        .map(|f| {
            let v: Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
            v["id"].as_str().unwrap_or_else(|| panic!("{}: no id", f.display())).to_owned()
        })
        .collect();
    let text = std::fs::read_to_string(root.join("conformance/coverage.tsv")).expect("conformance/coverage.tsv");
    let mut lines = text.lines().enumerate().filter(|(_, l)| !l.is_empty() && !l.starts_with('#'));
    assert_eq!(lines.next().map(|(_, l)| l), Some("ref\trule\tcover"), "the first line is the header");
    let (mut refs, mut items, mut errors) = (HashSet::new(), BTreeSet::new(), Vec::new());
    for (n, line) in lines {
        let n = n + 1;
        let fields: Vec<&str> = line.split('\t').collect();
        let [r, rule, cover] = fields[..] else {
            errors.push(format!("line {n}: {} fields, want 3", fields.len()));
            continue;
        };
        if !refs.insert(r.to_owned()) {
            errors.push(format!("line {n}: {r} repeated"));
        }
        let tokens: Vec<&str> = cover.split(' ').filter(|t| !t.is_empty()).collect();
        if tokens.iter().collect::<HashSet<_>>().len() != tokens.len() {
            errors.push(format!("line {n}: {r} names a token twice"));
        }
        let proven = tokens.iter().any(|t| t.starts_with("test:") || ids.contains(*t));
        if let Some(i) = r.strip_prefix("Edge case ") {
            match i.parse::<u32>() {
                Ok(e) if (1..=EDGE_CASES).contains(&e) && e.to_string() == i => {
                    items.insert(e);
                    if edges.get(&e).map(String::as_str) != Some(rule) {
                        errors.push(format!("line {n}: {r} does not read as SEMANTICS.md §Edge cases states it"));
                    }
                }
                _ => errors.push(format!("line {n}: {r} is not Edge case <1-{EDGE_CASES}>")),
            }
        } else if let Some((section, label)) = r.split_once(": ") {
            if section == SURFACES {
                if tokens.iter().any(|t| !OWNERS.contains(t) && !t.starts_with("test:")) {
                    errors.push(format!("line {n}: {r} is a surface's duty, so it names owners and tests, never cases"));
                }
            } else if !SECTIONS.contains(&section) {
                errors.push(format!("line {n}: {r} names no section the map covers"));
            } else if !proven {
                errors.push(format!("line {n}: {r} is a rule of the evaluator, so a case or a test proves it"));
            }
            if label.trim().is_empty() {
                errors.push(format!("line {n}: {r} has no label"));
            }
        } else {
            errors.push(format!("line {n}: {r}: want <section>: <label>, or Edge case <n>"));
        }
        if rule.trim().is_empty() {
            errors.push(format!("line {n}: no rule text"));
        }
        if tokens.is_empty() {
            errors.push(format!("line {n}: {r} names no proof and no owner"));
        }
        errors.extend(tokens.iter().filter_map(|t| token(t, &ids, root).err()).map(|e| format!("line {n}: {e}")));
    }
    let missing: Vec<u32> = (1..=EDGE_CASES).filter(|i| !items.contains(i)).collect();
    if !missing.is_empty() {
        errors.push(format!("edge cases with no line: {missing:?}"));
    }
    for s in SECTIONS {
        if !refs.iter().any(|r| r.strip_prefix(s).is_some_and(|rest| rest.starts_with(": "))) {
            errors.push(format!("{s} has no line"));
        }
    }
    assert!(errors.is_empty(), "{} coverage errors:\n{}", errors.len(), errors.join("\n"));
}
