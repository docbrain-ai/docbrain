// SPDX-License-Identifier: MIT
//! The conformance corpus runner: every case under `conformance/cases/` (format in `conformance/README.md`).
//! It runs each op this reader implements, fails on an unknown op or field, and prints every failing id.

use std::collections::HashSet;
use std::path::Path;

use docbrain_knowledge::bounds::CONTEXT_CAP;
use docbrain_knowledge::check::{CheckState, RecordState};
use docbrain_knowledge::folder::{SupersedeProblem, status};
use docbrain_knowledge::gate::{ChangeFinding, ChangeKind, Class, gate, gate_full};
use docbrain_knowledge::record::{Parsed, parse_record};
use docbrain_knowledge::route::context;
use serde_json::{Map, Value, json};

mod common;
use common::{mem_tree, tree, walk};

const OPS: [&str; 4] = ["parse", "status", "context", "gate"];
const FIELDS: [&str; 8] = ["id", "spec", "note", "op", "tree", "base", "paths", "expect"];
/// Ops whose reader is not built yet; their cases are counted, not run.
const PENDING: [&str; 0] = [];

/// A case's `spec` names what it proves: an edge case `SEMANTICS.md` lists (`Edge case 12`, `Edge cases 18 and
/// 67`), or a section of `SEMANTICS.md` by its heading, then the end, `:`, `,`, `;` or ` (`.
fn cites(spec: &str, sections: &[&str], edges: u32) -> bool {
    let number = |r: &str| r.split(|c: char| !c.is_ascii_digit()).next().and_then(|d| d.parse::<u32>().ok()).is_some_and(|n| (1..=edges).contains(&n));
    let edge = ["Edge case ", "Edge cases "].iter().any(|p| spec.strip_prefix(p).is_some_and(number));
    edge || sections.iter().any(|h| spec.strip_prefix(h).is_some_and(|r| r.is_empty() || [":", ",", ";", " ("].iter().any(|t| r.starts_with(t))))
}

fn check_json(c: &CheckState) -> Value {
    match c {
        CheckState::Holds(_) => json!({"state": "holds"}),
        CheckState::Broken(f) => json!({"state": "broken", "reason": f.code()}),
        CheckState::Stale(f) => json!({"state": "stale", "reason": f.code()}),
        CheckState::Unverifiable(u) => json!({"state": "unverifiable", "reason": u.code()}),
    }
}

/// `status` as the corpus writes it (`conformance/README.md`). A section left out of `expect` is empty.
/// `context` at the cap of `SEMANTICS.md` §Routing: the routed records' ids in order (a record without a readable id by its
/// path), and what the cap hides; `more` and `more_broken_or_stale` left out are 0.
fn run_context(case: &Value) -> Result<(), String> {
    let paths = case["paths"].as_array().ok_or("paths is not an array")?;
    let paths = paths.iter().map(|p| p.as_str().map(str::to_owned).ok_or("a path is not a string")).collect::<Result<Vec<_>, _>>()?;
    let report = context(&mem_tree(&case["tree"])?, &paths, CONTEXT_CAP).map_err(|e| e.to_string())?;
    let want = case["expect"].as_object().ok_or("expect is not an object")?;
    if let Some(k) = want.keys().find(|k| !["order", "more", "more_broken_or_stale"].contains(&k.as_str())) {
        return Err(format!("unknown expect key {k}"));
    }
    let want = json!({
        "order": want.get("order").ok_or("expect has no order")?,
        "more": want.get("more").unwrap_or(&json!(0)),
        "more_broken_or_stale": want.get("more_broken_or_stale").unwrap_or(&json!(0)),
    });
    let got = json!({
        "order": report.records.iter().map(|r| r.id.map_or_else(|| r.path.clone(), |i| i.to_string())).collect::<Vec<_>>(),
        "more": report.more,
        "more_broken_or_stale": report.more_broken_or_stale,
    });
    if got == want { Ok(()) } else { Err(format!("got {got}, want {want}")) }
}

fn problem_code(p: SupersedeProblem) -> &'static str {
    match p {
        SupersedeProblem::Itself => "itself",
        SupersedeProblem::NotInFolder => "not-in-folder",
        SupersedeProblem::Duplicated => "duplicated",
    }
}

/// `gate` (the reference form until the footprint gate lands) as the corpus writes it: a section left out of
/// `expect` is empty, `broken_before` left out is 0. A reason is compared by its code, the text before ": ".
fn run_gate(case: &Value) -> Result<(), String> {
    let (base, tree) = (mem_tree(&case["base"])?, mem_tree(&case["tree"])?);
    let report = gate_full(&base, &tree).map_err(|e| e.to_string())?;
    // The footprint form must give the reference's report (`SEMANTICS.md` §Cost).
    let footprint = gate(&base, &tree, &base.changed_paths(&tree)).map_err(|e| e.to_string())?;
    if footprint != report {
        return Err(format!("the footprint gate differs from the reference: {footprint:?} vs {report:?}"));
    }
    let want = case["expect"].as_object().ok_or("expect is not an object")?;
    const SECTIONS: [&str; 4] = ["findings", "change_findings", "broken_before", "knowledge"];
    if let Some(k) = want.keys().find(|k| !SECTIONS.contains(&k.as_str())) {
        return Err(format!("unknown expect section {k}"));
    }
    const KNOWLEDGE: [&str; 5] = ["added", "edited", "removed", "followed", "new_unverifiable"];
    let wk = want.get("knowledge").cloned().unwrap_or_else(|| json!({}));
    if let Some(k) = wk.as_object().ok_or("knowledge is not an object")?.keys().find(|k| !KNOWLEDGE.contains(&k.as_str())) {
        return Err(format!("unknown knowledge key {k}"));
    }
    let findings: Vec<Value> = report
        .findings
        .iter()
        .map(|f| {
            let mut o = Map::new();
            match &f.class {
                Class::Broken { rule } => {
                    o.insert("class".into(), json!("broken"));
                    o.insert("rule".into(), json!(rule));
                }
                Class::Changed(k) => {
                    o.insert("class".into(), json!("changed"));
                    o.insert("change".into(), json!(k.code()));
                    if let ChangeKind::AnchorMoved { from, to } | ChangeKind::ExpectationChanged { from, to } = k {
                        o.insert("from".into(), json!(from));
                        o.insert("to".into(), json!(to));
                    }
                }
                Class::Stale => {
                    o.insert("class".into(), json!("stale"));
                }
            }
            o.insert("id".into(), json!(f.id.to_string()));
            o.insert("record".into(), json!(f.record_path));
            o.insert("anchor".into(), json!(f.anchor));
            let codes: Vec<&str> = f.reasons.iter().map(|r| r.split(": ").next().unwrap_or(r)).collect();
            o.insert("reasons".into(), json!(codes));
            if let Some(s) = &f.same_content {
                o.insert("same_content".into(), json!(s));
            }
            Value::Object(o)
        })
        .collect();
    let change_findings: Vec<Value> = report
        .change_findings
        .iter()
        .map(|c| match c {
            ChangeFinding::DuplicateId { id, paths } => json!({"duplicate-id": {"id": id.to_string(), "paths": paths}}),
            ChangeFinding::Supersedes { path, problem } => json!({"supersedes": {"path": path, "problem": problem_code(*problem)}}),
            ChangeFinding::Cycle { ids } => json!({"cycle": ids.iter().map(ToString::to_string).collect::<Vec<_>>()}),
            ChangeFinding::SecretShape { path, rule, line, col } => json!({"secret-shape": {"path": path, "rule": rule, "line": line, "col": col}}),
        })
        .collect();
    let k = &report.knowledge;
    let got = json!({
        "findings": findings,
        "change_findings": change_findings,
        "broken_before": report.broken_before,
        "knowledge": {
            "added": k.added, "edited": k.edited, "removed": k.removed,
            "followed": k.followed.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
            "new_unverifiable": k.new_unverifiable.iter().map(|(p, i, r)| json!({"record": p, "check": i, "reason": r})).collect::<Vec<_>>(),
        },
    });
    let or_empty = |v: Option<&Value>| v.cloned().unwrap_or_else(|| json!([]));
    let want = json!({
        "findings": or_empty(want.get("findings")),
        "change_findings": or_empty(want.get("change_findings")),
        "broken_before": want.get("broken_before").cloned().unwrap_or_else(|| json!(0)),
        "knowledge": {
            "added": or_empty(wk.get("added")), "edited": or_empty(wk.get("edited")), "removed": or_empty(wk.get("removed")),
            "followed": or_empty(wk.get("followed")), "new_unverifiable": or_empty(wk.get("new_unverifiable")),
        },
    });
    if got == want { Ok(()) } else { Err(format!("got {got}, want {want}")) }
}

fn run_status(case: &Value) -> Result<(), String> {
    let report = status(&mem_tree(&case["tree"])?).map_err(|e| e.to_string())?;
    let want = case["expect"].as_object().ok_or("expect is not an object")?;
    const SECTIONS: [&str; 6] = ["records", "ignored", "duplicates", "problems", "cycles", "folder_wide"];
    if let Some(k) = want.keys().find(|k| !SECTIONS.contains(&k.as_str())) {
        return Err(format!("unknown expect section {k}"));
    }
    let mut records = Map::new();
    let mut wide = Vec::new();
    for r in &report.records {
        let mut o = Map::new();
        let (state, extra) = match &r.state {
            RecordState::Invalid { reason, .. } => ("invalid", Some(("reason", reason.code().into_owned()))),
            RecordState::Superseded { by } => ("superseded", Some(("by", by.to_string()))),
            RecordState::Broken => ("broken", None),
            RecordState::Stale => ("stale", None),
            RecordState::Unverifiable if r.checks.is_empty() => ("unverifiable", Some(("reason", "newer-schema".to_owned()))),
            RecordState::Unverifiable => ("unverifiable", None),
            RecordState::Holds => ("holds", None),
        };
        o.insert("state".into(), state.into());
        if let Some((k, v)) = extra {
            o.insert(k.into(), v.into());
        }
        o.insert("checks".into(), r.checks.iter().map(check_json).collect());
        if r.folder_wide {
            wide.push(Value::from(r.path.clone()));
        }
        records.insert(r.path.clone(), Value::Object(o));
    }
    let (mut ignored, mut duplicates, mut problems, mut cycles) = (Vec::new(), Map::new(), Map::new(), Vec::new());
    for f in &report.folders {
        ignored.extend(f.ignored.iter().cloned().map(Value::from));
        for (id, paths) in &f.duplicates {
            duplicates.insert(id.to_string(), paths.clone().into());
        }
        for (path, p) in &f.supersede_problems {
            let code = match p {
                SupersedeProblem::NotInFolder => "not-in-folder",
                SupersedeProblem::Itself => "itself",
                SupersedeProblem::Duplicated => "duplicated",
            };
            problems.insert(path.clone(), code.into());
        }
        cycles.extend(f.cycles.iter().map(|c| c.iter().map(|id| id.to_string()).collect::<Value>()));
    }
    ignored.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    let got = json!({
        "records": records, "ignored": ignored, "duplicates": duplicates,
        "problems": problems, "cycles": cycles, "folder_wide": wide,
    });
    let mut errors = Vec::new();
    for k in SECTIONS {
        let empty = if matches!(k, "records" | "duplicates" | "problems") { json!({}) } else { json!([]) };
        // A record's `checks` left out of the expectation are not compared; every other field is.
        let w = want.get(k).cloned().unwrap_or(empty);
        let mut g = got[k].clone();
        if k == "records" {
            for (path, r) in w.as_object().into_iter().flatten() {
                if r.get("checks").is_none()
                    && let Some(o) = g.get_mut(path).and_then(Value::as_object_mut)
                {
                    o.remove("checks");
                }
            }
        }
        if w != g {
            errors.push(format!("{k}: expected {w}, got {g}"));
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

/// the scope is the directory above the first `.knowledge/records/` on the path.
fn scope_of(path: &str) -> Option<String> {
    let segs: Vec<&str> = path.split('/').collect();
    let i = segs.windows(2).position(|w| w == [".knowledge", "records"])?;
    Some(segs[..i].join("/"))
}

fn run_parse(case: &Value) -> Result<(), String> {
    let t = tree(&case["tree"])?;
    let [(path, bytes)] = t.iter().collect::<Vec<_>>()[..] else { return Err("a parse case has exactly one file".into()) };
    let scope = scope_of(path).ok_or("the file is not under .knowledge/records/")?;
    let got = parse_record(&scope, bytes);
    let want = &case["expect"];
    let ok = match (&got, want.get("valid"), want.get("invalid"), want.get("newer")) {
        (Parsed::Valid(r), Some(Value::Bool(true)), None, None) => {
            let keys: Vec<&str> = want["unknown_keys"].as_array().ok_or("valid needs unknown_keys")?.iter().filter_map(Value::as_str).collect();
            r.unknown_keys == keys
        }
        (Parsed::Invalid { reason, line, col }, None, Some(code), None) => {
            Some(reason.code().as_ref()) == code.as_str()
                && want.get("line").is_none_or(|l| l.as_u64() == Some(u64::from(*line)))
                && want.get("col").is_none_or(|c| c.as_u64() == Some(u64::from(*col)))
        }
        (Parsed::Newer { schema, .. }, None, None, Some(n)) => n.as_u64() == Some(*schema),
        _ => false,
    };
    if ok { Ok(()) } else { Err(format!("expected {want}, got {got:?}")) }
}

#[test]
fn corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("conformance/cases");
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no cases under {}", root.display());
    let semantics = include_str!("../SEMANTICS.md");
    // The sections that state rules: not the list of sections, the edge cases (cited by number) or this corpus.
    let sections: Vec<&str> = semantics.lines().filter_map(|l| l.strip_prefix("## ")).filter(|h| !["Sections", "Edge cases", "Conformance"].contains(h)).collect();
    let edges = semantics.split("\n## Edge cases\n").nth(1).and_then(|s| s.split("\n## ").next()).expect("SEMANTICS.md has §Edge cases")
        .lines().filter_map(|l| l.split_once(". ").and_then(|(n, _)| n.parse::<u32>().ok())).max().expect("edge cases");
    let (mut ran, mut pending, mut failed, mut ids) = (0, 0, Vec::new(), HashSet::new());
    for f in &files {
        let name = f.strip_prefix(&root).unwrap_or(f).display().to_string();
        let case: Value = match std::fs::read_to_string(f).map_err(|e| e.to_string()).and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string())) {
            Ok(v) => v,
            Err(e) => {
                failed.push(format!("{name}: {e}"));
                continue;
            }
        };
        let id = case["id"].as_str().unwrap_or_default().to_owned();
        let shape = case.as_object().map_or(Err("not an object".to_owned()), |o| match o.keys().find(|k| !FIELDS.contains(&k.as_str())) {
            Some(k) => Err(format!("unknown field {k}")),
            None if id.is_empty() || !ids.insert(id.clone()) => Err("missing or repeated id".to_owned()),
            None if !case["spec"].as_str().is_some_and(|s| cites(s, &sections, edges)) => Err("spec must name a section of SEMANTICS.md or an edge case".to_owned()),
            None => Ok(()),
        });
        let op = case["op"].as_str().unwrap_or_default();
        let result = shape.and_then(|()| match op {
            "parse" => run_parse(&case).map(|()| ran += 1),
            "status" => run_status(&case).map(|()| ran += 1),
            "context" => run_context(&case).map(|()| ran += 1),
            "gate" => run_gate(&case).map(|()| ran += 1),
            o if PENDING.contains(&o) => {
                pending += 1;
                Ok(())
            }
            o if OPS.contains(&o) => Err(format!("op {o} has no runner")),
            o => Err(format!("unknown op {o:?}")),
        });
        if let Err(e) = result {
            failed.push(format!("{id} ({name}): {e}"));
        }
    }
    eprintln!("conformance: {} cases, {ran} ran, {pending} pending", files.len());
    assert!(failed.is_empty(), "{} failing cases:\n{}", failed.len(), failed.join("\n"));
}
