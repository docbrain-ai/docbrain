// SPDX-License-Identifier: MIT
//! `conformance/coverage.tsv` names, for every rule the design states, what proves it: corpus cases, a test of
//! this crate for a bound too large for a case, or the owner outside this crate (format in
//! `conformance/README.md`). This test keeps the map honest: every line names something that exists, and every
//! edge case of the design's §8 has its line.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Owners outside this crate; each token names the issue that owns the part.
const OWNERS: [&str; 4] = ["cli", "server", "job", "skill"];
/// The design's sections the map covers; each has at least one line.
const SECTIONS: [&str; 9] = ["§4.3", "§4.6", "§4.7", "§5.3", "§7.1", "§7.2", "§7.5", "§7.6", "§8"];
/// The design's §8 lists items 1 to this.
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

/// A token is a case id, `git` (git's own behaviour: no reader acts), `<owner>:#<issue>`, or
/// `test:<file>::<name>` for a `#[test]` function of this crate.
fn token(t: &str, ids: &HashSet<String>, root: &Path) -> Result<(), String> {
    if t == "git" || ids.contains(t) {
        return Ok(());
    }
    if let Some((owner, issue)) = t.split_once(":#") {
        let number = !issue.is_empty() && issue.bytes().all(|b| b.is_ascii_digit());
        return if OWNERS.contains(&owner) && number { Ok(()) } else { Err(format!("{t}: not an owner with an issue")) };
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
        let head = r.split(' ').next().unwrap_or_default();
        if let Some(i) = head.strip_prefix("§8.") {
            match i.parse::<u32>() {
                Ok(i) if (1..=EDGE_CASES).contains(&i) && head == r => {
                    items.insert(i);
                }
                _ => errors.push(format!("line {n}: {r} is not §8.<1-{EDGE_CASES}>")),
            }
        } else if !SECTIONS.contains(&head) || head == "§8" {
            errors.push(format!("line {n}: {r} names no section the map covers"));
        }
        if rule.trim().is_empty() {
            errors.push(format!("line {n}: no rule text"));
        }
        let tokens: Vec<&str> = cover.split(' ').filter(|t| !t.is_empty()).collect();
        if tokens.is_empty() {
            errors.push(format!("line {n}: {r} names no proof and no owner"));
        }
        errors.extend(tokens.iter().filter_map(|t| token(t, &ids, root).err()).map(|e| format!("line {n}: {e}")));
    }
    let missing: Vec<u32> = (1..=EDGE_CASES).filter(|i| !items.contains(i)).collect();
    if !missing.is_empty() {
        errors.push(format!("§8 items with no line: {missing:?}"));
    }
    for s in SECTIONS.iter().filter(|s| **s != "§8") {
        if !refs.iter().any(|r| r.split(' ').next() == Some(*s)) {
            errors.push(format!("{s} has no line"));
        }
    }
    assert!(errors.is_empty(), "{} coverage errors:\n{}", errors.len(), errors.join("\n"));
}
