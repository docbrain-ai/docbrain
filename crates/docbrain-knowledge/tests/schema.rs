// SPDX-License-Identifier: MIT
//! `schema/knowledge-1.schema.json` against the corpus: every valid `parse` case's front matter validates, and
//! every case invalid for a reason the schema can express (`kind`, `id`, `example-id`, `missing-key:*`,
//! `type:*`) fails it. The schema is a subset of the rules; `SEMANTICS.md` §The record is the whole.

use std::path::{Path, PathBuf};

use base64::Engine;
use docbrain_knowledge::yaml::{Mode, Node, NodeKind, Style, read_yaml};
use jsonschema::{Draft, JSONSchema};
use serde_json::{Map, Number, Value};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "json") {
            out.push(p);
        }
    }
}

/// The record's front matter as JSON: a plain all-digit scalar is a number (only `knowledge` is plain in a
/// record that parses), every other scalar a string.
fn to_json(n: &Node) -> Value {
    match &n.kind {
        NodeKind::Scalar { text, style } => match (style, text.parse::<u64>()) {
            (Style::Plain, Ok(v)) if text.bytes().all(|b| b.is_ascii_digit()) => Value::Number(Number::from(v)),
            _ => Value::String(text.clone()),
        },
        NodeKind::Seq(items) => Value::Array(items.iter().map(to_json).collect()),
        NodeKind::Map(entries) => {
            let mut m = Map::new();
            for (k, v) in entries {
                if let NodeKind::Scalar { text, .. } = &k.kind {
                    m.insert(text.clone(), to_json(v));
                }
            }
            Value::Object(m)
        }
    }
}

/// The front matter of a case's one file, or `None` when it has none that reads.
fn front_matter(case: &Value) -> Option<Value> {
    let (_, entry) = case["tree"].as_object()?.iter().next()?;
    let bytes = match entry {
        Value::String(s) => s.clone().into_bytes(),
        Value::Object(o) => base64::engine::general_purpose::STANDARD.decode(o.get("base64")?.as_str()?).ok()?,
        _ => return None,
    };
    let text = String::from_utf8(bytes).ok()?;
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text).replace("\r\n", "\n");
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---\n").map(|i| i + 1).or_else(|| rest.strip_suffix("\n---").map(|r| r.len() + 1))?;
    let docs = read_yaml(&rest[..end], Mode::Strict).ok()?;
    docs.first().map(to_json)
}

fn expressible(code: &str) -> bool {
    matches!(code, "kind" | "id" | "example-id") || code.starts_with("missing-key:") || code.starts_with("type:")
}

#[test]
fn schema_agrees_with_the_corpus() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(crate_dir.join("schema/knowledge-1.schema.json")).unwrap()).unwrap();
    assert_eq!(schema["$schema"], "https://json-schema.org/draft/2020-12/schema");
    let validator = JSONSchema::options().with_draft(Draft::Draft202012).compile(&schema).unwrap();
    assert!(!validator.is_valid(&serde_json::json!({})), "the schema rejects an empty object");

    let mut files = Vec::new();
    walk(&crate_dir.join("conformance/cases"), &mut files);
    files.sort();
    let (mut valid, mut invalid, mut failed) = (0, 0, Vec::new());
    for f in &files {
        let case: Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
        if case["op"] != "parse" {
            continue;
        }
        let id = case["id"].as_str().unwrap_or_default();
        let expect = &case["expect"];
        let code = expect["invalid"].as_str();
        if expect["valid"] != Value::Bool(true) && !code.is_some_and(expressible) {
            continue;
        }
        let Some(fm) = front_matter(&case) else {
            failed.push(format!("{id}: front matter does not convert"));
            continue;
        };
        let ok = validator.is_valid(&fm);
        match code {
            None if ok => valid += 1,
            None => failed.push(format!("{id}: valid record fails the schema: {fm}")),
            Some(_) if !ok => invalid += 1,
            Some(c) => failed.push(format!("{id}: invalid ({c}) record passes the schema: {fm}")),
        }
    }
    eprintln!("schema: {valid} valid cases validate, {invalid} invalid cases fail");
    assert!(valid > 0 && invalid > 0, "no case reached the schema: {valid} valid, {invalid} invalid");
    assert!(failed.is_empty(), "{} disagreements:\n{}", failed.len(), failed.join("\n"));
}
