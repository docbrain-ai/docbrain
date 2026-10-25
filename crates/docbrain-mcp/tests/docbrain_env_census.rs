// SPDX-License-Identifier: MIT
//! Guard (#603): the connector reads an env file in exactly one place, `load_docbrain_env`,
//! which only `--docbrain-env` reaches. Any other loader call — the implicit `dotenv()`, a
//! `from_filename` that walks parent directories, an `_override` or `_iter` variant, an
//! import that would let one be called unqualified — brings back the implicit load the owner
//! ruled out on 2026-10-23. It reads this crate's sources and the sources of every
//! crate it depends on by path, since their code runs in the same process.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// `src/` and `build.rs` of this crate and of each `path = "..."` dependency.
fn crate_sources(manifest_dir: &Path, seen: &mut Vec<PathBuf>, out: &mut Vec<PathBuf>) {
    let dir = manifest_dir.canonicalize().unwrap();
    if seen.contains(&dir) {
        return;
    }
    seen.push(dir.clone());
    rust_files(&dir.join("src"), out);
    if dir.join("build.rs").exists() {
        out.push(dir.join("build.rs"));
    }
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    for line in manifest.lines() {
        if let Some(rest) = line.split("path = \"").nth(1) {
            let rel = rest.split('"').next().unwrap();
            if !rel.ends_with(".rs") {
                crate_sources(&dir.join(rel), seen, out);
            }
        }
    }
}

#[test]
fn the_only_docbrain_env_load_is_the_flagged_one() {
    let mut files = Vec::new();
    crate_sources(Path::new(env!("CARGO_MANIFEST_DIR")), &mut Vec::new(), &mut files);
    assert!(files.iter().any(|f| f.ends_with("src/main.rs")), "no sources were found: {files:?}");
    let mut hits = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        for (i, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            // Naming the loader's error variants loads nothing.
            let code = code.replace("dotenvy::Error::", "");
            // Every occurrence counts, so a second call on the allowed line is caught too.
            for _ in code.matches("dotenv") {
                hits.push(format!("{}:{}: {}", f.display(), i + 1, code));
            }
        }
    }
    assert_eq!(hits.len(), 1, "expected exactly one env-file load (the --docbrain-env one), found:\n{}", hits.join("\n"));
    assert!(
        hits[0].contains("src/lib.rs:") && hits[0].contains("dotenvy::from_read_iter("),
        "the one load must be `dotenvy::from_read_iter` in `load_docbrain_env`, which checks every name before it sets any:\n{}",
        hits[0]
    );
}


/// The text between `// docbrain-env:begin` and `// docbrain-env:end` in a source file.
fn block(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap();
    let start = text.find("// docbrain-env:begin").unwrap_or_else(|| panic!("{}: no `// docbrain-env:begin`", path.display()));
    let end = text.find("// docbrain-env:end").unwrap_or_else(|| panic!("{}: no `// docbrain-env:end`", path.display()));
    text[start..end].to_string()
}

/// The table in the block: each DocBrain variable, and which binaries' files may set it.
fn table(block: &str) -> Vec<(String, bool, bool)> {
    block
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("(\"DOCBRAIN_"))
        .map(|l| {
            let name = l[2..].split('"').next().unwrap().to_string();
            (name, l.contains("Binary::Cli"), l.contains("Binary::Connector"))
        })
        .collect()
}

/// Every name the code reads with `env::var("...")` or `env::var_os("...")`: an actual read,
/// where a string in a message would not do.
fn env_reads(files: &[PathBuf]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for f in files {
        let text = std::fs::read_to_string(f).unwrap();
        for pat in ["env::var(\"", "env::var_os(\""] {
            for (i, _) in text.match_indices(pat) {
                let name: String = text[i + pat.len()..].chars().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_').collect();
                out.insert(name);
            }
        }
    }
    out
}

/// Every `"DOCBRAIN_..."` string literal in the code of these files (comments and the block
/// skipped): what the binary reads, or names, as one of DocBrain's variables.
fn docbrain_names(files: &[PathBuf]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for f in files {
        let text = std::fs::read_to_string(f).unwrap();
        let mut in_block = false;
        for line in text.lines() {
            let code = line.trim_start();
            if code.starts_with("// docbrain-env:begin") {
                in_block = true;
            } else if code.starts_with("// docbrain-env:end") {
                in_block = false;
            }
            if in_block || code.starts_with("//") {
                continue;
            }
            for (i, _) in code.match_indices("\"DOCBRAIN_") {
                let name: String = code[i + 1..].chars().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_').collect();
                out.insert(name);
            }
        }
    }
    out
}

/// The two binaries cannot share a crate, so each carries the table of DocBrain's variables and
/// who may set them from a `--docbrain-env` file (owner ruling 2026-10-24, an allow-list). It
/// must be the same table, byte for byte, between the markers.
#[test]
fn the_classification_is_the_same_in_both_binaries() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mcp = block(&root.join("docbrain-mcp/src/lib.rs"));
    let cli = block(&root.join("docbrain-cli/src/main.rs"));
    assert!(!table(&mcp).is_empty(), "the block holds no table:\n{mcp}");
    assert_eq!(mcp, cli, "the two tables differ");
}

/// Every DocBrain variable the connector names is classified, and every one the table lets the
/// connector's file set is one the connector reads: a new `DOCBRAIN_` variable fails here until
/// someone decides whether a named file may set it. Each entry is read by one binary at least.
#[test]
fn every_docbrain_variable_the_connector_reads_is_classified() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let rows = table(&block(&root.join("docbrain-mcp/src/lib.rs")));
    let mut mcp_files = Vec::new();
    crate_sources(&root.join("docbrain-mcp"), &mut Vec::new(), &mut mcp_files);
    let mut cli_files = Vec::new();
    crate_sources(&root.join("docbrain-cli"), &mut Vec::new(), &mut cli_files);
    let read = docbrain_names(&mcp_files);
    let read_by_either: std::collections::BTreeSet<String> = read.union(&docbrain_names(&cli_files)).cloned().collect();
    assert!(read.contains("DOCBRAIN_API_KEY"), "none of the connector's variables were found: {read:?}");
    for name in &read {
        assert!(rows.iter().any(|(n, _, _)| n == name), "{name} is read by the connector but not classified");
    }
    let reads = env_reads(&mcp_files);
    for (name, _, connector) in &rows {
        assert!(!connector || reads.contains(name), "{name} may be set in the connector's file but the connector never reads it with env::var");
        assert!(read_by_either.contains(name), "{name} is classified but neither binary reads it");
    }
}

/// The file is loaded in a synchronous `main`, before the async runtime is built, because
/// writing the environment is only sound while the process has one thread (the `# Safety` of
/// `load_docbrain_env`). Nothing else would notice the load moving into the runtime.
#[test]
fn the_file_is_loaded_before_the_async_runtime_starts() {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs")).unwrap();
    assert!(!text.contains("#[tokio::main]"), "main is async again");
    let main = text.find("\nfn main(").expect("a synchronous `fn main`");
    let body = &text[main..];
    let load = body.find("load_docbrain_env(").expect("main loads the file");
    let runtime = body.find("runtime::Builder").expect("main builds the runtime");
    assert!(load < runtime, "the file is loaded after the runtime is built");
    let asyncs: Vec<usize> = text.match_indices("\nasync fn ").map(|(i, _)| i).collect();
    for start in asyncs {
        let end = text[start + 1..].find("\n}\n").map(|e| start + 1 + e).unwrap_or(text.len());
        assert!(!text[start..end].contains("load_docbrain_env("), "an async fn loads the file");
    }
}
