// SPDX-License-Identifier: MIT
//! Every duty `docbrain-knowledge`'s coverage map gives the CLI (`conformance/coverage.tsv`, a cover naming `cli`)
//! is proven here by a named test, or named as a later part's (`fixtures/knowledge/cli-coverage.tsv`). This test
//! keeps that map honest: every CLI line is there, every test it names exists, and no line outlives its rule.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

/// The later parts of the CLI a duty may be left to.
const LATER: [&str; 3] = ["hook", "mcp", "job"];

/// The refs of the crate's map whose cover names `cli`, and every ref it has.
fn crate_refs(root: &Path) -> (BTreeSet<String>, HashSet<String>) {
    let path = root.join("../docbrain-knowledge/conformance/coverage.tsv");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let (mut cli, mut all) = (BTreeSet::new(), HashSet::new());
    for line in text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).skip(1) {
        let fields: Vec<&str> = line.split('\t').collect();
        let [r, _, cover] = fields[..] else { panic!("coverage.tsv: {line:?} has {} fields", fields.len()) };
        all.insert(r.to_owned());
        if cover.split(' ').any(|t| t == "cli") {
            cli.insert(r.to_owned());
        }
    }
    (cli, all)
}

#[test]
fn every_cli_row_is_proven() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let (cli, all) = crate_refs(root);
    assert!(cli.len() > 30, "the crate's map names cli on {} lines only", cli.len());
    let text = std::fs::read_to_string(root.join("tests/fixtures/knowledge/cli-coverage.tsv")).unwrap();
    let mut lines = text.lines().enumerate().filter(|(_, l)| !l.is_empty() && !l.starts_with('#'));
    assert_eq!(lines.next().map(|(_, l)| l), Some("ref\tcover"), "the first line is the header");
    let (mut seen, mut errors) = (BTreeSet::new(), Vec::new());
    for (n, line) in lines {
        let n = n + 1;
        let Some((r, cover)) = line.split_once('\t') else {
            errors.push(format!("line {n}: no cover"));
            continue;
        };
        if !seen.insert(r.to_owned()) {
            errors.push(format!("line {n}: {r} repeated"));
        }
        if !all.contains(r) {
            errors.push(format!("line {n}: {r} is not a line of coverage.tsv"));
        } else if !cli.contains(r) {
            errors.push(format!("line {n}: {r} names no cli duty in coverage.tsv"));
        }
        let tokens: Vec<&str> = cover.split(' ').filter(|t| !t.is_empty()).collect();
        if tokens.is_empty() {
            errors.push(format!("line {n}: {r} names nothing"));
        }
        for t in tokens {
            if let Some(plan) = t.strip_prefix("later:") {
                if !LATER.contains(&plan) {
                    errors.push(format!("line {n}: {t} is none of later:{}", LATER.join(", later:")));
                }
            } else if let Some((file, name)) = t.split_once("::") {
                match std::fs::read_to_string(root.join(file)) {
                    Ok(src) if src.contains(&format!("fn {name}(")) => {}
                    Ok(_) => errors.push(format!("line {n}: {file} has no fn {name}")),
                    Err(e) => errors.push(format!("line {n}: {file}: {e}")),
                }
            } else {
                errors.push(format!("line {n}: {t} is neither <file>::<fn> nor later:<plan>"));
            }
        }
    }
    for r in cli.difference(&seen) {
        errors.push(format!("{r}: a cli duty with no line"));
    }
    assert!(errors.is_empty(), "{} problems:\n{}", errors.len(), errors.join("\n"));
}
