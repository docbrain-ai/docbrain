// SPDX-License-Identifier: MIT
//! Commit trees through git (`SEMANTICS.md` §The gate, §Cost): the answers a MemTree gives, in a full clone and in a blobless
//! partial clone, with the fetches batched.

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use docbrain_knowledge::bounds::ANCHORED_READ_MAX;
use docbrain_knowledge::folder::{StatusReport, status};
use docbrain_knowledge::gate::{gate, gate_full};
use docbrain_knowledge::git::{GitError, GitRepo};
use docbrain_knowledge::{MemTree, Mode, Oid, Tree, TreeError};
use serde_json::Value;

mod common;
use common::{mem_tree, walk};

/// Runs git in `dir` with `env` added; panics on failure; stdout.
fn git_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let out = Command::new("git").current_dir(dir).args(args).envs(env.iter().copied()).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}
fn git(dir: &Path, args: &[&str]) -> String {
    git_env(dir, args, &[])
}

/// A repository built with `git fast-import`, each commit listing its whole tree.
struct Repo {
    dir: tempfile::TempDir,
    stream: Vec<u8>,
    marks: usize,
}

/// C-style quoting, as fast-import reads a path.
fn quote(p: &str) -> String {
    let mut q = String::from("\"");
    for b in p.bytes() {
        match b {
            b'"' => q.push_str("\\\""),
            b'\\' => q.push_str("\\\\"),
            0x20..=0x7e => q.push(b as char),
            _ => q.push_str(&format!("\\{b:03o}")),
        }
    }
    q.push('"');
    q
}

impl Repo {
    fn new() -> Repo {
        Repo::with_format("sha1")
    }
    fn with_format(format: &str) -> Repo {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", &format!("--object-format={format}")]);
        Repo { dir, stream: Vec::new(), marks: 0 }
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
    /// Queues a commit of `t` on `parents`; its mark.
    fn commit(&mut self, t: &MemTree, parents: &[usize]) -> usize {
        self.marks += 1;
        let m = self.marks;
        let s = &mut self.stream;
        writeln!(s, "commit refs/heads/c{m}\nmark :{m}\ncommitter t <t@example.com> {m} +0000\ndata 0").unwrap();
        for (i, p) in parents.iter().enumerate() {
            writeln!(s, "{} :{p}", if i == 0 { "from" } else { "merge" }).unwrap();
        }
        writeln!(s, "deleteall").unwrap();
        for (path, e) in t.list("").unwrap() {
            let mode = match e.mode {
                Mode::Submodule => {
                    writeln!(s, "M 160000 {} {}", e.oid, quote(&path)).unwrap();
                    continue;
                }
                Mode::File => "100644",
                Mode::Executable => "100755",
                Mode::Symlink => "120000",
            };
            let bytes = t.read_many(std::slice::from_ref(&e.oid)).unwrap().remove(0);
            writeln!(s, "M {mode} inline {}\ndata {}", quote(&path), bytes.len()).unwrap();
            s.extend_from_slice(&bytes);
            s.push(b'\n');
        }
        m
    }
    /// Imports the queued commits: oid by mark.
    fn import(&mut self) -> HashMap<usize, Oid> {
        let marks = self.path().join(".git/marks");
        let mut child = Command::new("git")
            .current_dir(self.path())
            .args(["fast-import", "--quiet", &format!("--export-marks={}", marks.display())])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&std::mem::take(&mut self.stream)).unwrap();
        assert!(child.wait().unwrap().success());
        std::fs::read_to_string(marks)
            .unwrap()
            .lines()
            .map(|l| {
                let (m, o) = l.split_once(' ').unwrap();
                (m.trim_start_matches(':').parse().unwrap(), Oid::from_hex(o).unwrap())
            })
            .collect()
    }
}

/// A blobless partial clone of `origin`, its fetches counted in the returned log.
fn blobless(origin: &Path) -> (tempfile::TempDir, PathBuf, PathBuf) {
    partial(origin, "blob:none")
}
fn partial(origin: &Path, filter: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    git(origin, &["config", "uploadpack.allowFilter", "true"]);
    git(origin, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
    let dir = tempfile::tempdir().unwrap();
    let clone = dir.path().join("clone");
    let url = format!("file://{}", origin.display());
    git(dir.path(), &["clone", "-q", &format!("--filter={filter}"), "--no-checkout", &url, clone.to_str().unwrap()]);
    let log = dir.path().join("fetches.log");
    let wrapper = dir.path().join("upload-pack.sh");
    std::fs::write(&wrapper, format!("#!/bin/sh\necho fetch >> '{}'\nexec git upload-pack \"$@\"\n", log.display())).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    git(&clone, &["config", "remote.origin.uploadpack", wrapper.to_str().unwrap()]);
    (dir, clone, log)
}

fn fetches(log: &Path) -> usize {
    std::fs::read_to_string(log).map_or(0, |s| s.lines().count())
}

/// Every corpus case: (id, op, case).
fn cases() -> Vec<(String, String, Value)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("conformance/cases");
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    files
        .iter()
        .map(|f| {
            let c: Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
            (c["id"].as_str().unwrap().to_owned(), c["op"].as_str().unwrap().to_owned(), c)
        })
        .collect()
}

/// A status report with record oids blanked: git names blobs by sha1, a MemTree by sha256.
fn blank(mut r: StatusReport) -> StatusReport {
    let zero = Oid::from_hex("00").unwrap();
    for f in &mut r.folders {
        for rec in &mut f.records {
            rec.oid = zero.clone();
        }
    }
    r
}

/// Paths and modes: what a git tree and a MemTree share (sizes and oids differ by design).
fn shape(v: Vec<(String, docbrain_knowledge::Entry)>) -> Vec<(String, Mode)> {
    v.into_iter().map(|(p, e)| (p, e.mode)).collect()
}

#[test]
fn commit_tree_matches_memtree() {
    // Every tree the corpus writes, as a commit: the same status, the same entries, every directory listed.
    let mut trees = Vec::new();
    for (id, _, c) in cases() {
        for k in ["tree", "base"] {
            if c.get(k).is_some_and(Value::is_object) {
                trees.push((format!("{id}/{k}"), mem_tree(&c[k]).unwrap()));
            }
        }
    }
    assert!(trees.len() > 300, "{} trees", trees.len());
    let mut r = Repo::new();
    let marks: Vec<usize> = trees.iter().map(|(_, t)| r.commit(t, &[])).collect();
    let oids = r.import();
    let repo = GitRepo::discover(r.path()).unwrap();
    for ((id, t), m) in trees.iter().zip(marks) {
        let ct = repo.commit_tree(&oids[&m]).unwrap();
        let st = |x: &dyn Tree| status(x).map(blank).map_err(|e| e.to_string());
        assert_eq!(st(&ct), st(t), "{id}");
        assert_eq!(shape(ct.list("").unwrap()), shape(t.list("").unwrap()), "{id}");
        let mut dirs: Vec<String> = Vec::new();
        for (p, _) in t.list("").unwrap() {
            let mut at = p.as_str();
            while let Some((d, _)) = at.rsplit_once('/') {
                dirs.push(d.to_owned());
                at = d;
            }
            let (g, w) = (ct.entry(&p).unwrap().unwrap(), t.entry(&p).unwrap().unwrap());
            assert_eq!(g.mode, w.mode, "{id} {p}");
            if g.mode != Mode::Submodule {
                assert_eq!(ct.read_many(&[g.oid]).unwrap(), t.read_many(&[w.oid]).unwrap(), "{id} {p}");
            }
        }
        dirs.sort();
        dirs.dedup();
        for d in dirs {
            assert_eq!(ct.entry(&d).unwrap(), None, "{id} {d}");
            assert_eq!(shape(ct.list(&d).unwrap()), shape(t.list(&d).unwrap()), "{id} {d}");
        }
    }
}

#[test]
fn changed_paths_no_renames() {
    let mut b = MemTree::new();
    b.file("a.txt", "same\n").file("keep", "k\n").file("m", "x\n").file("d/f", "1\n").file("sub", "s\n");
    let mut h = MemTree::new();
    h.file("b.txt", "same\n").file("keep", "k\n").exec("m", "x\n").file("d", "now a file\n").submodule("sub", "4d7a2c1f00000000000000000000000000000000");
    let mut r = Repo::new();
    let mb = r.commit(&b, &[]);
    let mh = r.commit(&h, &[mb]);
    let o = r.import();
    git(r.path(), &["config", "diff.renames", "copies"]);
    let repo = GitRepo::discover(r.path()).unwrap();
    let got = repo.changed_paths(&o[&mb], &o[&mh]).unwrap();
    assert_eq!(got.paths(), ["a.txt", "b.txt", "d", "d/f", "m", "sub"]);
    assert_eq!(got, b.changed_paths(&h));
    assert!(repo.changed_paths(&o[&mb], &o[&mb]).unwrap().paths().is_empty());
}

#[test]
fn symlink_and_submodule_modes() {
    let mut t = MemTree::new();
    t.file("d/f", "x").exec("run.sh", "#!/bin/sh\n").symlink("link", "d/f").symlink("dl", "d").submodule("sub", "4d7a2c1f00000000000000000000000000000000");
    let mut r = Repo::new();
    let m = r.commit(&t, &[]);
    let o = r.import();
    let ct = GitRepo::discover(r.path()).unwrap().commit_tree(&o[&m]).unwrap();
    let mode = |p: &str| ct.entry(p).unwrap().map(|e| e.mode);
    assert_eq!(mode("link"), Some(Mode::Symlink));
    assert_eq!(mode("dl"), Some(Mode::Symlink));
    assert_eq!(mode("sub"), Some(Mode::Submodule));
    assert_eq!(mode("run.sh"), Some(Mode::Executable));
    assert_eq!(mode("d/f"), Some(Mode::File));
    assert_eq!(ct.entry("sub").unwrap().unwrap().oid, Oid::from_hex("4d7a2c1f00000000000000000000000000000000").unwrap());
    // Nothing is followed: not a link, not a submodule, and a directory or the root is not an entry.
    for p in ["dl/f", "sub/x", "link/x", "d", "", "d/", "d//f", "./d/f", "d/./f"] {
        assert_eq!(mode(p), None, "{p:?}");
    }
    for p in ["dl", "sub", "link", "d/f", "nope"] {
        assert!(ct.list(p).unwrap().is_empty(), "{p}");
    }
    let link = ct.entry("link").unwrap().unwrap();
    assert_eq!(&*ct.read_many(std::slice::from_ref(&link.oid)).unwrap()[0], b"d/f");
    // In argument order, duplicates kept.
    let f = ct.entry("d/f").unwrap().unwrap().oid;
    let got = ct.read_many(&[f.clone(), link.oid, f]).unwrap();
    assert_eq!(got.iter().map(|b| &b[..]).collect::<Vec<_>>(), [&b"x"[..], b"d/f", b"x"]);
    // Git keeps no size in a tree: every bound is judged on the bytes read.
    assert_eq!(ct.entry("run.sh").unwrap().unwrap().size, 0);
}

#[test]
fn pathspec_magic_is_literal() {
    let names = ["*.md", "a.md", ":(glob)x", ":x", "[ab]/f", "a/f", "b/f", "x", "sp ace/ta\tb", "new\nline", "caf\u{e9}/q\"uote\\"];
    let mut t = MemTree::new();
    for (i, n) in names.iter().enumerate() {
        t.file(n, format!("{i}\n"));
    }
    let mut r = Repo::new();
    let m = r.commit(&t, &[]);
    let o = r.import();
    let ct = GitRepo::discover(r.path()).unwrap().commit_tree(&o[&m]).unwrap();
    for (i, n) in names.iter().enumerate() {
        let e = ct.entry(n).unwrap().unwrap_or_else(|| panic!("{n:?}"));
        assert_eq!(&*ct.read_many(&[e.oid]).unwrap()[0], format!("{i}\n").as_bytes(), "{n:?}");
    }
    for n in ["?.md", "*", ":(glob)*", "[ab]", "[ab]/*", "a.m?"] {
        assert_eq!(ct.entry(n).unwrap(), None, "{n:?}");
    }
    assert_eq!(shape(ct.list("[ab]").unwrap()), vec![("[ab]/f".to_owned(), Mode::File)]);
    assert_eq!(shape(ct.list("a").unwrap()), vec![("a/f".to_owned(), Mode::File)]);
    assert_eq!(shape(ct.list("").unwrap()), shape(t.list("").unwrap()));
}

#[test]
fn several_merge_bases() {
    let one = |s: &str| {
        let mut t = MemTree::new();
        t.file("f", s);
        t
    };
    let mut r = Repo::new();
    let r0 = r.commit(&one("0"), &[]);
    let a1 = r.commit(&one("a1"), &[r0]);
    let b1 = r.commit(&one("b1"), &[r0]);
    let a2 = r.commit(&one("a2"), &[a1, b1]);
    let b2 = r.commit(&one("b2"), &[b1, a1]);
    let lone = r.commit(&one("lone"), &[]);
    let o = r.import();
    let repo = GitRepo::discover(r.path()).unwrap();
    assert_eq!(repo.merge_base(&o[&a1], &o[&b1]).unwrap(), o[&r0]);
    match repo.merge_base(&o[&a2], &o[&b2]) {
        Err(GitError::SeveralBases(mut v)) => {
            v.sort();
            let mut want = vec![o[&a1].clone(), o[&b1].clone()];
            want.sort();
            assert_eq!(v, want);
        }
        other => panic!("{other:?}"),
    }
    // The message names every base.
    let msg = repo.merge_base(&o[&a2], &o[&b2]).unwrap_err().to_string();
    let named = |x: &Oid, y: &Oid| msg == format!("several merge bases: {x}, {y}");
    assert!(named(&o[&a1], &o[&b1]) || named(&o[&b1], &o[&a1]), "{msg}");
    assert!(matches!(repo.merge_base(&o[&a1], &o[&lone]), Err(GitError::NoMergeBase)), "unrelated");
    // Revisions: names, oids, never options; a tree is not a commit.
    assert_eq!(repo.resolve_commit(&format!("refs/heads/c{a2}")).unwrap(), o[&a2]);
    assert_eq!(repo.resolve_commit(&format!("c{b2}~1")).unwrap(), o[&b1]);
    assert_eq!(repo.resolve_commit(o[&a2].as_str()).unwrap(), o[&a2]);
    let tree = git(r.path(), &["rev-parse", &format!("c{a2}^{{tree}}")]);
    for bad in ["nope", "--all", "-h", "", tree.trim()] {
        assert!(matches!(repo.resolve_commit(bad), Err(GitError::NotFound(_))), "{bad:?}");
    }
    let tree = Oid::from_hex(tree.trim()).unwrap();
    assert!(matches!(repo.commit_tree(&tree), Err(GitError::NotFound(_))));
    assert!(matches!(repo.commit_tree(&Oid::from_hex(&"ab".repeat(20)).unwrap()), Err(GitError::NotFound(_))));
    // From a directory inside the work tree.
    std::fs::create_dir(r.path().join("sub")).unwrap();
    assert_eq!(GitRepo::discover(&r.path().join("sub")).unwrap().resolve_commit(o[&a2].as_str()).unwrap(), o[&a2]);
    // Not a repository.
    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(GitRepo::discover(empty.path()), Err(GitError::NotARepo)));
    // A repository git refuses for another reason says so, not "not a repository".
    let odd = tempfile::tempdir().unwrap();
    git(odd.path(), &["init", "-q"]);
    git(odd.path(), &["config", "core.repositoryformatversion", "1"]);
    git(odd.path(), &["config", "extensions.zzunknown", "yes"]);
    match GitRepo::discover(odd.path()) {
        Err(GitError::Git { stderr, .. }) => assert!(stderr.contains("zzunknown"), "{stderr}"),
        other => panic!("{other:?}"),
    }
}

/// A read asks for blobs: a commit or a tree under a blob's name is refused, and the next read still works.
#[test]
fn read_many_reads_only_blobs() {
    let mut t = MemTree::new();
    t.file("d/f", "x");
    let mut r = Repo::new();
    let c = r.commit(&t, &[]);
    let o = r.import();
    let ct = GitRepo::discover(r.path()).unwrap().commit_tree(&o[&c]).unwrap();
    let tree = Oid::from_hex(git(r.path(), &["rev-parse", &format!("{}^{{tree}}", o[&c])]).trim()).unwrap();
    for (oid, ty) in [(o[&c].clone(), "commit"), (tree, "tree")] {
        match ct.read_many(std::slice::from_ref(&oid)) {
            Err(TreeError::Git { stderr, .. }) => assert_eq!(stderr, format!("{oid} is a {ty}, not a blob")),
            other => panic!("{other:?}"),
        }
    }
    let e = ct.entry("d/f").unwrap().unwrap();
    assert_eq!(&*ct.read_many(&[e.oid]).unwrap()[0], b"x");
}

/// Three folders of records, each check reading its own file; H edits one of svc/a's files.
fn fleet() -> (MemTree, MemTree) {
    let mut b = MemTree::new();
    for (s, scope) in ["", "svc/a/", "svc/b/"].iter().enumerate() {
        for i in 0..8 {
            let rec = format!(
                "---\nknowledge: 1\nid: 'k3v9q2x7m{s}{i:02}'\nkind: 'fact'\ntitle: 't{s}{i}'\nchecks:\n  - config_key: 'conf/v{i}.yaml#replicas'\n    must: '>= {n}'\n---\nWhy.\n",
                n = i + 1
            );
            b.file(&format!("{scope}.knowledge/records/r{i}.md"), rec).file(&format!("{scope}conf/v{i}.yaml"), format!("replicas: {}\n", i + 1));
        }
    }
    // A record past its bound: invalid, read only because git keeps no size.
    b.file("svc/a/.knowledge/records/big.md", format!("---\nknowledge: 1\n{}", " ".repeat(70_000)));
    let mut h = b.clone();
    h.file("svc/a/conf/v3.yaml", "replicas: 0\n");
    (b, h)
}

#[test]
fn blobless_clone_fetches_in_one_batch() {
    let (b, h) = fleet();
    let mut origin = Repo::new();
    let mb = origin.commit(&b, &[]);
    let mh = origin.commit(&h, &[mb]);
    let o = origin.import();
    let (_keep, clone, log) = blobless(origin.path());
    let repo = GitRepo::discover(&clone).unwrap();
    let changed = repo.changed_paths(&o[&mb], &o[&mh]).unwrap();
    assert_eq!(changed, b.changed_paths(&h));
    assert_eq!(fetches(&log), 0, "the changed paths read no blob");
    let (cb, ch) = (repo.commit_tree(&o[&mb]).unwrap(), repo.commit_tree(&o[&mh]).unwrap());
    let got = gate(&cb, &ch, &changed).unwrap();
    assert_eq!(got, gate_full(&b, &h).unwrap());
    assert_eq!(got.findings.len(), 1, "{got:?}");
    // About twenty blobs per tree, fetched in batches: records, then the judged checks' files.
    let n = fetches(&log);
    assert!((1..=4).contains(&n), "{n} fetches");
    // Nothing outside the folder above the change was fetched.
    let absent = git(&clone, &["rev-parse", &format!("{}:svc/b/.knowledge/records/r0.md", o[&mb])]);
    let st = Command::new("git").current_dir(&clone).args(["cat-file", "-e", absent.trim()]).env("GIT_NO_LAZY_FETCH", "1").status().unwrap();
    assert!(!st.success(), "svc/b's record was fetched");
}

/// Status fetches a glob's files in one batch in a blobless clone, and a pass over blobs already held fetches none.
#[test]
fn a_glob_is_fetched_in_one_batch() {
    let mut t = MemTree::new();
    t.file(".knowledge/records/a.md", "---\nknowledge: 1\nid: 'k3v9q2x7m1ab'\nkind: 'fact'\ntitle: 't'\nchecks:\n  - config_key: 'c/*.json#k'\n    must: 'exist'\n---\nWhy.\n");
    for i in 0..6 {
        t.file(&format!("c/f{i}.json"), format!("{{\"k\": {i}}}"));
    }
    let mut origin = Repo::new();
    let m = origin.commit(&t, &[]);
    let o = origin.import();
    let (_keep, clone, log) = blobless(origin.path());
    let want = blank(status(&t).unwrap());
    let ct = GitRepo::discover(&clone).unwrap().commit_tree(&o[&m]).unwrap();
    assert_eq!(blank(status(&ct).unwrap()), want);
    assert_eq!(fetches(&log), 2, "the record, then the glob's six files");
    let again = GitRepo::discover(&clone).unwrap().commit_tree(&o[&m]).unwrap();
    assert_eq!(blank(status(&again).unwrap()), want);
    assert_eq!(fetches(&log), 2, "every blob is held");
}

/// The gate names the files its judged checks read to the tree together (`SEMANTICS.md` §Cost): one fetch per
/// tree for six files, and an anchor on a submodule is never named, or the whole batch would be refused.
#[test]
fn the_gate_fetches_a_trees_inputs_in_one_batch() {
    let keys: String = (0..6).map(|i| format!("  - config_key: 'c/f{i}.json#k'\n    must: '== 1'\n")).collect();
    let rec = format!("---\nknowledge: 1\nid: 'k3v9q2x7m1ab'\nkind: 'fact'\ntitle: 't'\nchecks:\n{keys}  - code: 'sub'\n    text: 'x'\n    must: 'exist'\n---\nWhy.\n");
    let mut b = MemTree::new();
    b.file(".knowledge/records/a.md", rec).submodule("sub", "4d7a2c1f00000000000000000000000000000000");
    for i in 0..6 {
        b.file(&format!("c/f{i}.json"), "{\"k\": 1}");
    }
    let mut h = b.clone();
    for i in 0..6 {
        h.file(&format!("c/f{i}.json"), format!("{{\"k\": {}}}", i + 2));
    }
    // The submodule moves too, so its check is judged in both trees and its anchor reaches the filter.
    h.submodule("sub", "9e1b5f3a00000000000000000000000000000000");
    let mut origin = Repo::new();
    let mb = origin.commit(&b, &[]);
    let mh = origin.commit(&h, &[mb]);
    let o = origin.import();
    let (_keep, clone, log) = blobless(origin.path());
    let repo = GitRepo::discover(&clone).unwrap();
    let changed = repo.changed_paths(&o[&mb], &o[&mh]).unwrap();
    let (cb, ch) = (repo.commit_tree(&o[&mb]).unwrap(), repo.commit_tree(&o[&mh]).unwrap());
    let got = gate(&cb, &ch, &changed).unwrap();
    assert_eq!(got, gate_full(&b, &h).unwrap());
    assert_eq!(got.findings.len(), 6, "{got:?}");
    assert_eq!(fetches(&log), 3, "the record, then B's six files, then H's six");
}

#[test]
fn offline_missing_object() {
    let mut t = MemTree::new();
    t.file(".knowledge/records/a.md", "---\nknowledge: 1\nid: 'k3v9q2x7m1ab'\nkind: 'fact'\ntitle: 't'\n---\nWhy.\n");
    let mut origin = Repo::new();
    let m = origin.commit(&t, &[]);
    let o = origin.import();
    let (keep, clone, log) = blobless(origin.path());
    let gone = keep.path().join("gone");
    git(&clone, &["remote", "set-url", "origin", &format!("file://{}", gone.display())]);
    let ct = GitRepo::discover(&clone).unwrap().commit_tree(&o[&m]).unwrap();
    // Trees are held: the entry is there, its blob is not.
    let e = ct.entry(".knowledge/records/a.md").unwrap().unwrap();
    match ct.read_many(std::slice::from_ref(&e.oid)) {
        Err(TreeError::MissingObject(x)) => assert_eq!(x, e.oid),
        other => panic!("{other:?}"),
    }
    assert!(matches!(status(&ct), Err(TreeError::MissingObject(_))));
    assert!(fetches(&log) >= 1, "a fetch was tried");
}

#[test]
fn read_cuts_past_the_bound() {
    let n = ANCHORED_READ_MAX as usize + 10;
    let mut big = vec![b'a'; n];
    big[0] = b'z';
    let mut t = MemTree::new();
    t.file("big.json", &big).file("small", "s");
    let mut origin = Repo::new();
    let m = origin.commit(&t, &[]);
    let o = origin.import();
    let read = |ct: &dyn Tree| {
        let (e, s) = (ct.entry("big.json").unwrap().unwrap(), ct.entry("small").unwrap().unwrap());
        let got = ct.read_many(&[e.oid, s.oid]).unwrap();
        assert_eq!(got[0].len(), ANCHORED_READ_MAX as usize + 1);
        assert_eq!(got[0][..4], *b"zaaa");
        assert_eq!(&*got[1], b"s");
    };
    read(&GitRepo::discover(origin.path()).unwrap().commit_tree(&o[&m]).unwrap());
    let (_keep, clone, log) = blobless(origin.path());
    // A promisor remote whose name has dots, and `extensions.partialClone` naming a remote that is gone.
    git(&clone, &["remote", "rename", "origin", "up.stream"]);
    git(&clone, &["config", "extensions.partialClone", "gone"]);
    read(&GitRepo::discover(&clone).unwrap().commit_tree(&o[&m]).unwrap());
    assert_eq!(fetches(&log), 1);
}

#[test]
fn treeless_clone_fetches_trees() {
    let mut t = MemTree::new();
    t.file("a/b/c.txt", "c\n").file("a/d.txt", "d\n").symlink("l", "a").file("top", "t\n");
    let mut origin = Repo::new();
    let m = origin.commit(&t, &[]);
    let o = origin.import();
    let (_keep, clone, log) = partial(origin.path(), "tree:0");
    let ct = GitRepo::discover(&clone).unwrap().commit_tree(&o[&m]).unwrap();
    assert_eq!(shape(ct.list("").unwrap()), shape(t.list("").unwrap()));
    assert_eq!(ct.entry("a/b").unwrap(), None);
    let e = ct.entry("a/b/c.txt").unwrap().unwrap();
    assert_eq!(&*ct.read_many(&[e.oid]).unwrap()[0], b"c\n");
    let n = fetches(&log);
    assert!((1..=4).contains(&n), "{n} fetches");
}

#[test]
fn sha256_repository() {
    let mut t = MemTree::new();
    t.file("a/f", "1\n").exec("x", "2\n").symlink("l", "a/f");
    let mut r = Repo::with_format("sha256");
    let (mb, mut h) = (r.commit(&t, &[]), t.clone());
    h.file("a/f", "3\n");
    let mh = r.commit(&h, &[mb]);
    let o = r.import();
    assert_eq!(o[&mb].as_str().len(), 64);
    let repo = GitRepo::discover(r.path()).unwrap();
    let ct = repo.commit_tree(&o[&mh]).unwrap();
    assert_eq!(shape(ct.list("").unwrap()), shape(t.list("").unwrap()));
    let e = ct.entry("a/f").unwrap().unwrap();
    assert_eq!(e.oid.as_str().len(), 64);
    assert_eq!(&*ct.read_many(&[e.oid]).unwrap()[0], b"3\n");
    assert_eq!(repo.changed_paths(&o[&mb], &o[&mh]).unwrap().paths(), ["a/f"]);
    assert_eq!(repo.merge_base(&o[&mb], &o[&mh]).unwrap(), o[&mb]);
}

#[test]
fn gate_on_commits_equals_memtree() {
    // Every gate case as two commits: git's changed paths are the MemTree's, and the gate on commit trees gives
    // the reference report, which the corpus runner checks against each case's expectation.
    let gates: Vec<(String, MemTree, MemTree)> =
        cases().into_iter().filter(|(_, op, _)| op == "gate").map(|(id, _, c)| (id, mem_tree(&c["base"]).unwrap(), mem_tree(&c["tree"]).unwrap())).collect();
    assert!(gates.len() > 50, "{} gate cases", gates.len());
    let mut r = Repo::new();
    let marks: Vec<(usize, usize)> = gates.iter().map(|(_, b, h)| (r.commit(b, &[]), r.commit(h, &[]))).collect();
    let o = r.import();
    let repo = GitRepo::discover(r.path()).unwrap();
    for ((id, b, h), (mb, mh)) in gates.iter().zip(marks) {
        let changed = repo.changed_paths(&o[&mb], &o[&mh]).unwrap();
        assert_eq!(changed, b.changed_paths(h), "{id}");
        let (cb, ch) = (repo.commit_tree(&o[&mb]).unwrap(), repo.commit_tree(&o[&mh]).unwrap());
        let want = gate_full(b, h).map_err(|e| e.to_string());
        assert_eq!(gate(&cb, &ch, &changed).map_err(|e| e.to_string()), want, "{id}");
        assert_eq!(gate_full(&cb, &ch).map_err(|e| e.to_string()), want, "{id}");
    }
}

/// Every file under `dir`, sorted.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

#[test]
fn tree_by_id_reads_a_written_tree() {
    let r = Repo::new();
    std::fs::write(r.path().join("a.yaml"), "x: 1\n").unwrap();
    git(r.path(), &["add", "a.yaml"]);
    let t = Oid::from_hex(git(r.path(), &["write-tree"]).trim()).unwrap();
    let tree = GitRepo::discover(r.path()).unwrap().tree(&t).unwrap();
    let e = tree.entry("a.yaml").unwrap().unwrap();
    assert_eq!(&*tree.read_many(&[e.oid]).unwrap()[0], b"x: 1\n");
}

#[test]
fn tree_by_id_refuses_a_commit_id() {
    let mut t = MemTree::new();
    t.file("a.yaml", "x: 1\n");
    let mut r = Repo::new();
    let c = r.commit(&t, &[]);
    let o = r.import();
    match GitRepo::discover(r.path()).unwrap().tree(&o[&c]) {
        Err(GitError::NotFound(name)) => assert_eq!(name, o[&c].as_str()),
        other => panic!("{:?}", other.map(|_| ())),
    }
}

#[test]
fn overlay_reads_objects_written_only_there() {
    let mut t = MemTree::new();
    t.file("a.yaml", "x: 1\n");
    let mut r = Repo::new();
    r.commit(&t, &[]);
    r.import();
    let before = files_under(&r.path().join(".git/objects"));
    let overlay = tempfile::tempdir().unwrap();
    let objects = r.path().join(".git/objects");
    let env = [("GIT_OBJECT_DIRECTORY", overlay.path().to_str().unwrap()), ("GIT_ALTERNATE_OBJECT_DIRECTORIES", objects.to_str().unwrap())];
    let mut w = Command::new("git")
        .current_dir(r.path())
        .args(["hash-object", "-w", "--stdin"])
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    w.stdin.take().unwrap().write_all(b"only in the overlay\n").unwrap();
    let blob = String::from_utf8(w.wait_with_output().unwrap().stdout).unwrap().trim().to_owned();
    let mut m = Command::new("git").current_dir(r.path()).args(["mktree"]).envs(env).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    m.stdin.take().unwrap().write_all(format!("100644 blob {blob}\tnew.txt\n").as_bytes()).unwrap();
    let tree = Oid::from_hex(String::from_utf8(m.wait_with_output().unwrap().stdout).unwrap().trim()).unwrap();

    let repo = GitRepo::discover(r.path()).unwrap();
    assert!(matches!(repo.tree(&tree), Err(GitError::NotFound(_) | GitError::MissingObject(_))), "the repository alone does not hold it");
    let seen = repo.with_object_overlay(overlay.path()).unwrap().tree(&tree).unwrap();
    let e = seen.entry("new.txt").unwrap().unwrap();
    assert_eq!(&*seen.read_many(&[e.oid]).unwrap()[0], b"only in the overlay\n");
    assert_eq!(files_under(&objects), before, "nothing was written to the repository's objects");
}

#[test]
fn overlay_still_reads_the_repository() {
    let mut t = MemTree::new();
    t.file("a.yaml", "x: 1\n");
    t.file("d/b.md", "b\n");
    let mut r = Repo::new();
    let c = r.commit(&t, &[]);
    let o = r.import();
    let repo = GitRepo::discover(r.path()).unwrap();
    let overlay = tempfile::tempdir().unwrap();
    let plain = repo.commit_tree(&o[&c]).unwrap();
    let through = repo.with_object_overlay(overlay.path()).unwrap().commit_tree(&o[&c]).unwrap();
    assert_eq!(shape(through.list("").unwrap()), shape(plain.list("").unwrap()));
    let e = through.entry("d/b.md").unwrap().unwrap();
    assert_eq!(&*through.read_many(&[e.oid]).unwrap()[0], b"b\n");
    assert!(files_under(overlay.path()).is_empty(), "a read writes nothing to the overlay");
}

/// `:` separates alternates (`;` on Windows), a leading `"` quotes one: a repository whose path holds them is still
/// read through the overlay.
#[cfg(unix)]
#[test]
fn overlay_reads_a_repository_whose_path_holds_separators() {
    let scratch = tempfile::tempdir().unwrap();
    let dir = scratch.path().join("a:b;c\"d\\e");
    std::fs::create_dir(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    std::fs::write(dir.join("a.yaml"), "x: 1\n").unwrap();
    git(&dir, &["add", "a.yaml"]);
    git(&dir, &["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-q", "-m", "first"]);
    let repo = GitRepo::discover(&dir).unwrap();
    let head = repo.resolve_commit("HEAD").unwrap();
    let overlay = tempfile::tempdir().unwrap();
    let through = repo.with_object_overlay(overlay.path()).unwrap().commit_tree(&head).unwrap();
    let e = through.entry("a.yaml").unwrap().unwrap();
    assert_eq!(&*through.read_many(&[e.oid]).unwrap()[0], b"x: 1\n");
}
