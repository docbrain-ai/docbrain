// SPDX-License-Identifier: MIT
//! A repository at scale, and what the gate and `context` cost on it (`SEMANTICS.md` §Cost). `gate_bench <scratch-dir>`
//! builds, once, a bare repository of 1,020,000 files with `git fast-import`: 40,000 directories five levels
//! deep, 25 files each, and in every tenth one a `config.yaml` and a folder of five records, each checking
//! one key of that `config.yaml` (20,000 records). `main` is that tree; `head` changes one key of one
//! `config.yaml`; `wide` changes 5,500 files, one per directory in 5,000 of them. Then, best of five, each as a CLI run would do it from a fresh process's view (new trees, new
//! `git cat-file`): the gate on each change (merge base, changed paths, both trees), and `context` for
//! one path and for 5,000 (edge case 80). Each prints its time and git's share: the time inside the repository calls and the tree reads,
//! which is git's work and the crate's tree parsing; the rest is the evaluator.

use std::cell::Cell;
use std::error::Error;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use docbrain_knowledge::bounds::CONTEXT_CAP;
use docbrain_knowledge::git::GitRepo;
use docbrain_knowledge::{Entry, Oid, Tree, TreeError, gate, route};

const TOP: usize = 10;
const LEAF: usize = 4;
const DIRS: usize = TOP * TOP * TOP * TOP * LEAF;
const FILES_PER_DIR: usize = 25;
const RECORDS_PER_FOLDER: usize = 5;
const PLAIN_BLOBS: usize = 1_000;
const WIDE_DIRS: usize = 5_000;

fn dir(l: usize) -> String {
    let (e, rest) = (l % LEAF, l / LEAF);
    let (d, rest) = (rest % TOP, rest / TOP);
    let (c, rest) = (rest % TOP, rest / TOP);
    let (b, a) = (rest % TOP, rest / TOP);
    format!("a{a}/b{b}/c{c}/d{d}/e{e}")
}

/// Twelve characters of the record id alphabet, from `n`.
fn id(n: usize) -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut out = vec![b'k'];
    let mut v = n;
    for _ in 0..11 {
        out.push(ALPHABET[v % 32]);
        v /= 32;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn config(l: usize, k2: usize) -> String {
    format!("k0: 0\nk1: 1\nk2: {k2}\nk3: 3\nk4: 4\nd: {l}\n")
}

fn record(g: usize, j: usize) -> String {
    format!(
        "---\nknowledge: 1\nid: '{}'\nkind: 'fact'\ntitle: 'r{j}'\napplies_to: ['**']\nchecks:\n  - config_key: 'config.yaml#k{j}'\n    must: '== {j}'\n---\nWhy this matters.\n",
        id(g)
    )
}

fn blob(w: &mut impl Write, mark: usize, data: &str) -> std::io::Result<()> {
    write!(w, "blob\nmark :{mark}\ndata {}\n{data}\n", data.len())
}

/// The fast-import stream: plain blobs `1..=1000`, configs from 1001, records after them, the changed config,
/// then `main` and `head`.
fn build(git_dir: &Path) -> Result<(), Box<dyn Error>> {
    let status = Command::new("git").args(["init", "-q", "--bare"]).arg(git_dir).status()?;
    if !status.success() {
        return Err("git init failed".into());
    }
    let mut child = Command::new("git").arg("--git-dir").arg(git_dir).args(["fast-import", "--quiet"]).stdin(Stdio::piped()).spawn()?;
    let stdin = child.stdin.take().ok_or("no stdin")?;
    let mut w = std::io::BufWriter::new(stdin);
    let folders = DIRS / 10;
    let (config0, record0) = (PLAIN_BLOBS + 1, PLAIN_BLOBS + 1 + folders);
    let changed = record0 + folders * RECORDS_PER_FOLDER;
    for k in 0..PLAIN_BLOBS {
        blob(&mut w, k + 1, &format!("// file {k}\n{}\n", "x".repeat(200)))?;
    }
    for r in 0..folders {
        blob(&mut w, config0 + r, &config(r * 10, 2))?;
        for j in 0..RECORDS_PER_FOLDER {
            let g = r * RECORDS_PER_FOLDER + j;
            blob(&mut w, record0 + g, &record(g, j))?;
        }
    }
    blob(&mut w, changed, &config(0, 3))?;
    write!(w, "commit refs/heads/main\nmark :{}\ncommitter bench <bench@example.invalid> 0 +0000\ndata 1\nB\n", changed + 1)?;
    let mut files = 0;
    for l in 0..DIRS {
        let d = dir(l);
        let folder = l % 10 == 0;
        let plain = if folder { FILES_PER_DIR - 1 } else { FILES_PER_DIR };
        for i in 0..plain {
            writeln!(w, "M 100644 :{} {d}/f{l}_{i}.txt", (l * FILES_PER_DIR + i) % PLAIN_BLOBS + 1)?;
        }
        files += plain;
        if folder {
            let r = l / 10;
            writeln!(w, "M 100644 :{} {d}/config.yaml", config0 + r)?;
            for j in 0..RECORDS_PER_FOLDER {
                writeln!(w, "M 100644 :{} {d}/.knowledge/records/r{j}.md", record0 + r * RECORDS_PER_FOLDER + j)?;
            }
            files += 1 + RECORDS_PER_FOLDER;
        }
    }
    write!(w, "commit refs/heads/head\nmark :{}\ncommitter bench <bench@example.invalid> 0 +0000\ndata 1\nH\nfrom :{}\n", changed + 2, changed + 1)?;
    writeln!(w, "M 100644 :{changed} {}/config.yaml", dir(0))?;
    // `wide`: one file in each of the first 5,000 directories, and the `config.yaml` of the 500 folders among
    // them with only `d` changed, so every one of their checks is affected and still holds.
    let wide0 = changed + 3;
    blob(&mut w, wide0, "changed\n")?;
    for r in 0..WIDE_DIRS / 10 {
        blob(&mut w, wide0 + 1 + r, &config(r * 10 + 1_000_000, 2))?;
    }
    let wide = wide0 + 1 + WIDE_DIRS / 10;
    write!(w, "commit refs/heads/wide\nmark :{wide}\ncommitter bench <bench@example.invalid> 0 +0000\ndata 1\nW\nfrom :{}\n", changed + 1)?;
    for l in 0..WIDE_DIRS {
        writeln!(w, "M 100644 :{wide0} {}/f{l}_0.txt", dir(l))?;
        if l % 10 == 0 {
            writeln!(w, "M 100644 :{} {}/config.yaml", wide0 + 1 + l / 10, dir(l))?;
        }
    }
    w.flush()?;
    drop(w);
    if !child.wait()?.success() {
        return Err("git fast-import failed".into());
    }
    println!("built: {files} files, {} records", folders * RECORDS_PER_FOLDER);
    Ok(())
}

/// A tree that adds the time of every call to `spent`.
struct Timed<'a> {
    inner: &'a dyn Tree,
    spent: &'a Cell<Duration>,
}

impl Timed<'_> {
    fn time<T>(&self, f: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let out = f();
        self.spent.set(self.spent.get() + started.elapsed());
        out
    }
}

impl Tree for Timed<'_> {
    fn entry(&self, path: &str) -> Result<Option<Entry>, TreeError> {
        self.time(|| self.inner.entry(path))
    }
    fn list(&self, dir: &str) -> Result<Vec<(String, Entry)>, TreeError> {
        self.time(|| self.inner.list(dir))
    }
    fn read_many(&self, oids: &[Oid]) -> Result<Vec<Arc<[u8]>>, TreeError> {
        self.time(|| self.inner.read_many(oids))
    }
}

/// One run: the total and git's share, and what the run found.
type Run = (Duration, Duration, String);

fn gate_run(git_dir: &Path, head: &str) -> Result<Run, Box<dyn Error>> {
    let spent = Cell::new(Duration::ZERO);
    let mut steps: Vec<(&str, Duration)> = Vec::new();
    let mut at = Instant::now();
    let mut step = |name: &'static str| {
        let now = Instant::now();
        steps.push((name, now - at));
        at = now;
    };
    let started = Instant::now();
    let repo = GitRepo::discover(git_dir)?;
    step("discover");
    let (main, hd) = (repo.resolve_commit("refs/heads/main")?, repo.resolve_commit(head)?);
    step("resolve x2");
    let mb = repo.merge_base(&main, &hd)?;
    step("merge-base");
    let changed = repo.changed_paths(&mb, &hd)?;
    step("diff-tree");
    let (b, h) = (repo.commit_tree(&mb)?, repo.commit_tree(&hd)?);
    step("trees x2");
    let report = gate::gate(&Timed { inner: &b, spent: &spent }, &Timed { inner: &h, spent: &spent }, &changed)?;
    drop((b, h, repo));
    step("gate");
    let took = started.elapsed();
    let git: Duration = steps.iter().filter(|s| s.0 != "gate").map(|s| s.1).sum::<Duration>() + spent.get();
    let parts: Vec<String> = steps.iter().map(|(n, d)| format!("{n} {:.1}", d.as_secs_f64() * 1e3)).collect();
    let found = format!(
        "{} findings, {} changed paths; ms: {} (of which tree reads {:.1})",
        report.findings.len(),
        changed.paths().len(),
        parts.join(", "),
        spent.get().as_secs_f64() * 1e3
    );
    Ok((took, git, found))
}

fn context_run(git_dir: &Path, paths: &[String]) -> Result<Run, Box<dyn Error>> {
    let spent = Cell::new(Duration::ZERO);
    let started = Instant::now();
    let tree = {
        let r = GitRepo::discover(git_dir)?;
        let hd = r.resolve_commit("refs/heads/head")?;
        r.commit_tree(&hd)?
    };
    spent.set(started.elapsed());
    let report = route::context(&Timed { inner: &tree, spent: &spent }, paths, CONTEXT_CAP)?;
    drop(tree);
    let took = started.elapsed();
    Ok((took, spent.get(), format!("{} records, {} more", report.records.len(), report.more)))
}

fn best(name: &str, run: impl Fn() -> Result<Run, Box<dyn Error>>) -> Result<(), Box<dyn Error>> {
    let mut all = Vec::new();
    for _ in 0..5 {
        all.push(run()?);
    }
    let times: Vec<String> = all.iter().map(|r| format!("{:.1}", r.0.as_secs_f64() * 1e3)).collect();
    let (took, git, found) = all.into_iter().min_by_key(|r| r.0).ok_or("no run")?;
    let share = git.as_secs_f64() / took.as_secs_f64() * 100.0;
    println!(
        "{name}: best of 5 {:.1} ms (runs {} ms); git {:.1} ms ({share:.0}%), evaluator {:.1} ms; {found}",
        took.as_secs_f64() * 1e3,
        times.join(", "),
        git.as_secs_f64() * 1e3,
        (took - git).as_secs_f64() * 1e3
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let Some(scratch) = std::env::args().nth(1) else {
        eprintln!("usage: gate_bench <scratch-dir>");
        std::process::exit(2);
    };
    let git_dir = Path::new(&scratch).join("scale.git");
    if !git_dir.join("refs/heads/wide").exists() && !git_dir.join("packed-refs").exists() {
        let started = Instant::now();
        build(&git_dir)?;
        println!("built in {:.1} s", started.elapsed().as_secs_f64());
    }
    best("gate, one-file change", || gate_run(&git_dir, "refs/heads/head"))?;
    best("gate, 5,500-file change", || gate_run(&git_dir, "refs/heads/wide"))?;
    best("context, one path", || context_run(&git_dir, &[format!("{}/f0_3.txt", dir(0))]))?;
    // Edge case 80: 5,000 paths across 5,000 directories, 500 of them with a folder of records.
    let many: Vec<String> = (0..5_000).map(|l| format!("{}/f{l}_0.txt", dir(l))).collect();
    best("context, 5,000 paths", || context_run(&git_dir, &many))?;
    Ok(())
}
