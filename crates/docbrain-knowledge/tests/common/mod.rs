// SPDX-License-Identifier: MIT
//! Shared by the corpus runner and the git tests: case files, and the tree form a case writes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use base64::Engine;
use docbrain_knowledge::MemTree;
use serde_json::Value;

/// Every `.json` file under `dir`.
pub fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "json") {
            out.push(p);
        }
    }
}

/// A tree as the case writes it: path → bytes (a symlink or submodule entry has no bytes here).
pub fn tree(v: &Value) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let obj = v.as_object().ok_or("tree is not an object")?;
    let mut out = BTreeMap::new();
    for (path, entry) in obj {
        let bytes = match entry {
            Value::String(s) => s.clone().into_bytes(),
            Value::Object(o) if o.len() == 1 => match o.iter().next() {
                Some((k, Value::String(s))) if k == "base64" => {
                    base64::engine::general_purpose::STANDARD.decode(s).map_err(|e| format!("{path}: base64: {e}"))?
                }
                Some((k, Value::String(s))) if k == "exec" => s.clone().into_bytes(),
                Some((k, Value::String(_))) if k == "symlink" || k == "submodule" => Vec::new(),
                _ => return Err(format!("{path}: unknown entry form")),
            },
            _ => return Err(format!("{path}: unknown entry form")),
        };
        out.insert(path.clone(), bytes);
    }
    Ok(out)
}

/// The same tree with its modes, for the ops that read a tree.
pub fn mem_tree(v: &Value) -> Result<MemTree, String> {
    let obj = v.as_object().ok_or("tree is not an object")?;
    let bytes = tree(v)?;
    let mut t = MemTree::new();
    for (path, entry) in obj {
        match entry.as_object().and_then(|o| o.iter().next()) {
            Some((k, Value::String(s))) if k == "exec" => t.exec(path, s),
            Some((k, Value::String(s))) if k == "symlink" => t.symlink(path, s),
            Some((k, Value::String(s))) if k == "submodule" => t.submodule(path, s),
            _ => t.file(path, &bytes[path]),
        };
    }
    Ok(t)
}
