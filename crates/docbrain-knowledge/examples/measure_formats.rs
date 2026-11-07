// SPDX-License-Identifier: MIT
//! The anchored-file readers over real files, to set the bounds from measurement (design §7.5: each value
//! is recorded with its measurement). `measure_formats <dir>…` walks the directories, never into `.git` and
//! never through a symbolic link, and reads every file `format_of` accepts with its format's reader. Per
//! format it prints the count, the largest size, the read time p50, p95 and max with the size of the slowest
//! file, the failures by reason and how many of them are templates (the text holds `{{`), and for YAML the
//! heaviest charge for anchors and aliases; then the count and largest size of
//! the other files, which only a `code` anchor reads. It never prints a path, a key or a value.
//!
//! `measure_formats --resolve <file>` times what a bound promises, end to end: `folder::status` over a tree
//! holding the file and one record whose check reads it, best of three, for a `config_key` check when the file
//! has a format and for a `code` check always. It prints each check's state and time, never the text.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use docbrain_knowledge::check::CheckState;
use docbrain_knowledge::format::{Format, format_of, read_hcl, read_json, read_toml, read_yaml_file};
use docbrain_knowledge::yaml::alias_expansion;
use docbrain_knowledge::{MemTree, folder};

#[derive(Default)]
struct Stats {
    reads: Vec<(Duration, u64)>,
    largest: u64,
    failures: BTreeMap<String, usize>,
    templated: usize,
    alias_max: usize,
}

fn read(format: Format, text: &str) -> Result<(), String> {
    let r = match format {
        Format::Yaml => read_yaml_file(text).map(drop),
        Format::Json => read_json(text).map(drop),
        Format::Toml => read_toml(text).map(drop),
        Format::Hcl => read_hcl(text).map(drop),
    };
    r.map_err(|u| u.code().into_owned())
}

fn walk(dir: &Path, stats: &mut BTreeMap<&'static str, Stats>, other: &mut (usize, u64)) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.is_dir() {
            if e.file_name() != ".git" {
                walk(&path, stats, other);
            }
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(format) = format_of(&name) else {
            other.0 += 1;
            other.1 = other.1.max(meta.len());
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let s = stats.entry(label(format)).or_default();
        s.largest = s.largest.max(meta.len());
        let Ok(text) = std::str::from_utf8(&bytes) else {
            *s.failures.entry("encoding".to_owned()).or_default() += 1;
            continue;
        };
        let started = Instant::now();
        let result = read(format, text);
        s.reads.push((started.elapsed(), meta.len()));
        if let Err(code) = result {
            *s.failures.entry(code).or_default() += 1;
            s.templated += usize::from(text.contains("{{"));
        }
        if format == Format::Yaml
            && let Ok(n) = alias_expansion(text.strip_prefix('\u{feff}').unwrap_or(text))
        {
            s.alias_max = s.alias_max.max(n);
        }
    }
}

fn label(f: Format) -> &'static str {
    match f {
        Format::Yaml => "yaml",
        Format::Json => "json",
        Format::Toml => "toml",
        Format::Hcl => "hcl",
    }
}

fn mib(b: u64) -> String {
    format!("{:.2} MiB", b as f64 / 1_048_576.0)
}

/// One record checking `name` with `check`, beside the file, and the best of three `status` runs.
fn resolve(name: &str, bytes: &[u8], check: &str) -> (String, Duration) {
    let record = format!("---\nknowledge: 1\nid: 'k3v9q2x7m1aa'\nkind: 'fact'\ntitle: 'measure'\nchecks:\n{check}---\nWhy.\n");
    let mut tree = MemTree::new();
    tree.file(name, bytes).file(".knowledge/records/a.md", record);
    let mut best = (String::new(), Duration::MAX);
    for _ in 0..3 {
        let started = Instant::now();
        let report = folder::status(&tree);
        let took = started.elapsed();
        let state = match report.as_ref().ok().and_then(|r| r.records.first()).and_then(|r| r.checks.first()) {
            Some(CheckState::Holds(_)) => "holds".to_owned(),
            Some(CheckState::Broken(_)) => "broken".to_owned(),
            Some(CheckState::Stale(_)) => "stale".to_owned(),
            Some(CheckState::Unverifiable(u)) => format!("unverifiable {}", u.code()),
            None => format!("no check ({report:?})"),
        };
        if took < best.1 {
            best = (state, took);
        }
    }
    best
}

fn resolve_mode(file: &str) {
    let Ok(bytes) = std::fs::read(file) else {
        eprintln!("cannot read the file");
        std::process::exit(2);
    };
    let name = Path::new(file).file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let size = mib(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
    if let Some(f) = format_of(&name) {
        let check = format!("  - config_key: '{name}#x-measure'\n    must: 'exist'\n");
        let (state, took) = resolve(&name, &bytes, &check);
        println!("{}\t{size}\tconfig_key {state} in {took:?}", label(f));
    }
    let check = format!("  - code: '{name}'\n    text: 'measure-code-anchor-text-0123456789'\n    must: 'exist'\n");
    let (state, took) = resolve(&name, &bytes, &check);
    println!("code\t{size}\tcode {state} in {took:?}");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [mode, file] = &args[..]
        && mode == "--resolve"
    {
        return resolve_mode(file);
    }
    let dirs = args;
    if dirs.is_empty() {
        eprintln!("usage: measure_formats <dir>...");
        std::process::exit(2);
    }
    let (mut stats, mut other) = (BTreeMap::new(), (0, 0));
    for d in &dirs {
        walk(Path::new(d), &mut stats, &mut other);
    }
    for (name, s) in &mut stats {
        s.reads.sort();
        let n = s.reads.len();
        let at = |q: usize| s.reads.get((n * q / 100).min(n.saturating_sub(1))).map_or(Duration::ZERO, |r| r.0);
        let (max, size) = s.reads.last().copied().unwrap_or_default();
        let failures: Vec<String> = s.failures.iter().map(|(k, v)| format!("{k} {v}")).collect();
        let alias = if *name == "yaml" { format!("  alias expansion max {}", s.alias_max) } else { String::new() };
        println!(
            "{name}\tcount {n}\tlargest {}\tread p50 {:?} p95 {:?} max {max:?} (a {} file)\tfailures [{}], {} templated{alias}",
            mib(s.largest),
            at(50),
            at(95),
            mib(size),
            failures.join(", "),
            s.templated
        );
    }
    println!("other\tcount {}\tlargest {}", other.0, mib(other.1));
}
