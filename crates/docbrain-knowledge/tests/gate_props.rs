// SPDX-License-Identifier: MIT
//! The footprint gate equals the reference gate (`SEMANTICS.md` §Cost), and reads only what the change touched.

use std::cell::RefCell;

use docbrain_knowledge::gate::{gate, gate_full};
use docbrain_knowledge::{Entry, MemTree, Mode, Oid, Tree, TreeError};
use proptest::prelude::*;

const T: &str = "retry(3, backoff::exponential());";
/// The root folder, one nested, and its sibling.
const SCOPES: [&str; 3] = ["", "svc/a", "svc/b"];
/// Anchors, relative to the folder's directory. The last three hold by `v.yaml` with a layer under `d` that never
/// exists, so a link replacing `d` changes nothing they match (C14 needs the predicate's "above" clauses); the
/// one before them reaches into a nested folder from the root.
const ANCHORS: [&str; 13] = [
    "path: 'v.yaml'",
    "path: 'd/x.txt'",
    "path: 'd'",
    "path: 'd/*.txt'",
    "path: 'nope.txt'",
    "config_key: 'v.yaml#replicas'",
    "config_key: 'c.json#replicas'",
    "config_key:\n      - 'v.yaml#replicas'\n      - 'c.json#replicas'",
    "code: 'main.rs'\n    text: 'retry(3, backoff::exponential());'",
    "config_key: 'svc/a/v.yaml#replicas'",
    "config_key:\n      - 'v.yaml#replicas'\n      - 'd/p.yaml#replicas'",
    "config_key:\n      - 'v.yaml#replicas'\n      - 'd/*.yaml#replicas'",
    "config_key:\n      - 'v.yaml#replicas'\n      - 'd/e/*.yaml#replicas'",
];
const EXPECTS: [&str; 4] = ["exist", ">= 2", "== 1", "< 3"];
const KINDS: [&str; 5] = ["decision", "fact", "procedure", "question", "constraint"];
/// The data files of a folder's directory.
const FILES: [&str; 4] = ["v.yaml", "c.json", "main.rs", "d/x.txt"];

fn at(scope: &str, rel: &str) -> String {
    if scope.is_empty() { rel.to_owned() } else { format!("{scope}/{rel}") }
}

#[derive(Debug, Clone)]
struct Chk {
    anchor: usize,
    must: bool,
    expect: usize,
}

#[derive(Debug, Clone)]
struct Rec {
    id: u8,
    kind: usize,
    newer: bool,
    supersedes: Option<u8>,
    checks: Vec<Chk>,
}

fn id(n: u8) -> String {
    format!("k3v9q2x7m{n:03}")
}

fn record_text(r: &Rec) -> String {
    let mut s = format!("---\nknowledge: {}\nid: '{}'\nkind: '{}'\ntitle: 't{}'\n", if r.newer { 2 } else { 1 }, id(r.id), KINDS[r.kind], r.id);
    if let Some(t) = r.supersedes {
        s += &format!("supersedes: '{}'\n", id(t));
    }
    if !r.checks.is_empty() {
        s += "checks:\n";
        for c in &r.checks {
            s += &format!("  - {}\n    {}: '{}'\n", ANCHORS[c.anchor], if c.must { "must" } else { "assumes" }, EXPECTS[c.expect]);
        }
    }
    s + "---\nWhy this matters.\n\nMore.\n"
}

fn file_text(file: usize, value: u8) -> String {
    match (file, value) {
        (0, 5) => "replicas: '{{ .Values.n }}'\n".to_owned(),
        (0, v) => format!("replicas: {v}\n"),
        (1, v) => format!("{{\"replicas\": {}}}\n", v.min(4)),
        (2, v) if v % 2 == 0 => format!("fn main() {{\n    {T}\n}}\n"),
        (2, _) => "fn main() {}\n".to_owned(),
        _ => "x\n".to_owned(),
    }
}

#[derive(Debug, Clone)]
struct FolderSpec {
    records: Vec<Rec>,
    /// Each data file, absent or with a value.
    files: [Option<u8>; 4],
}

fn chk() -> impl Strategy<Value = Chk> {
    (0..ANCHORS.len(), any::<bool>(), 0..EXPECTS.len()).prop_map(|(anchor, must, expect)| Chk { anchor, must, expect })
}

fn rec() -> impl Strategy<Value = Rec> {
    (1u8..=4, prop::sample::select(vec![0usize, 0, 1, 2, 3, 4]), prop::bool::weighted(0.08), prop::option::weighted(0.2, 1u8..=4), prop::collection::vec(chk(), 0..=3))
        .prop_map(|(id, kind, newer, supersedes, checks)| Rec { id, kind, newer, supersedes, checks })
}

fn folder() -> impl Strategy<Value = Option<FolderSpec>> {
    prop::option::weighted(
        0.85,
        (prop::collection::vec(rec(), 0..=6), [prop::option::weighted(0.8, 0u8..=5), prop::option::weighted(0.6, 0u8..=5), prop::option::weighted(0.6, 0u8..=5), prop::option::weighted(0.6, 0u8..=5)])
            .prop_map(|(records, files)| FolderSpec { records, files }),
    )
}

fn tree_strategy() -> impl Strategy<Value = MemTree> {
    [folder(), folder(), folder()].prop_map(|folders| {
        let mut t = MemTree::new();
        for (scope, f) in SCOPES.iter().zip(folders) {
            let Some(f) = f else { continue };
            for (k, r) in f.records.iter().enumerate() {
                t.file(&at(scope, &format!(".knowledge/records/r{k}.md")), record_text(r));
            }
            for (n, v) in f.files.iter().enumerate() {
                if let Some(v) = v {
                    t.file(&at(scope, FILES[n]), file_text(n, *v));
                }
            }
        }
        t
    })
}

#[derive(Debug, Clone)]
enum Edit {
    SetValue { scope: usize, file: usize, value: u8 },
    DeleteFile { scope: usize, file: usize },
    MoveFile { scope: usize, file: usize, to: usize, rewrite: bool },
    MoveFolder { scope: usize, to: usize, rewrite: bool },
    Rewrite { pick: usize, how: usize },
    Supersede { pick: usize, scope: usize },
    DeleteRecord { pick: usize },
    CopyRecord { pick: usize, scope: usize },
    DirToSymlink { scope: usize, records: bool },
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        3 => (0..3usize, 0..2usize, 0u8..=5).prop_map(|(scope, file, value)| Edit::SetValue { scope, file, value }),
        2 => (0..3usize, 0..4usize).prop_map(|(scope, file)| Edit::DeleteFile { scope, file }),
        2 => (0..3usize, 0..4usize, 0..2usize, any::<bool>()).prop_map(|(scope, file, to, rewrite)| Edit::MoveFile { scope, file, to, rewrite }),
        1 => (1..3usize, 0..2usize, any::<bool>()).prop_map(|(scope, to, rewrite)| Edit::MoveFolder { scope, to, rewrite }),
        3 => (any::<usize>(), 0..8usize).prop_map(|(pick, how)| Edit::Rewrite { pick, how }),
        1 => (any::<usize>(), 0..3usize).prop_map(|(pick, scope)| Edit::Supersede { pick, scope }),
        1 => any::<usize>().prop_map(|pick| Edit::DeleteRecord { pick }),
        1 => (any::<usize>(), 0..3usize).prop_map(|(pick, scope)| Edit::CopyRecord { pick, scope }),
        1 => (0..3usize, any::<bool>()).prop_map(|(scope, records)| Edit::DirToSymlink { scope, records }),
    ]
}

fn edits_strategy() -> impl Strategy<Value = Vec<Edit>> {
    prop::collection::vec(edit(), 1..=4)
}

fn bytes(t: &MemTree, e: &Entry) -> Vec<u8> {
    t.read_many(std::slice::from_ref(&e.oid)).expect("read").first().map(|b| b.to_vec()).unwrap_or_default()
}

/// Every entry under `prefix` (a directory, with its trailing `/`) moved to `to`, modes and contents kept.
fn move_prefix(t: &mut MemTree, prefix: &str, to: &str) {
    for (p, e) in t.list("").expect("list") {
        if let Some(rest) = p.strip_prefix(prefix) {
            let content = bytes(t, &e);
            t.remove(&p);
            put(t, &format!("{to}{rest}"), &e, &content);
        }
    }
}

fn put(t: &mut MemTree, path: &str, e: &Entry, content: &[u8]) {
    match e.mode {
        Mode::File => t.file(path, content),
        Mode::Executable => t.exec(path, content),
        Mode::Symlink => t.symlink(path, &String::from_utf8_lossy(content)),
        Mode::Submodule => t.submodule(path, "0123456789abcdef0123456789abcdef01234567"),
    };
}

fn record_paths(t: &MemTree) -> Vec<(String, Entry)> {
    t.list("").expect("list").into_iter().filter(|(p, e)| p.contains(".knowledge/records/") && p.ends_with(".md") && e.mode != Mode::Symlink).collect()
}

fn apply(b: &MemTree, edits: &[Edit]) -> MemTree {
    let mut h = b.clone();
    for e in edits {
        match *e {
            Edit::SetValue { scope, file, value } => {
                h.file(&at(SCOPES[scope], FILES[file]), file_text(file, value));
            }
            Edit::DeleteFile { scope, file } => {
                h.remove(&at(SCOPES[scope], FILES[file]));
            }
            Edit::MoveFile { scope, file, to, rewrite } => {
                let from = at(SCOPES[scope], FILES[file]);
                let Some(en) = h.entry(&from).expect("entry") else { continue };
                let name = ["moved/", "w-"][to];
                let rel = format!("{name}{}", FILES[file]);
                let content = bytes(&h, &en);
                h.remove(&from);
                put(&mut h, &at(SCOPES[scope], &rel), &en, &content);
                if rewrite {
                    let dir = at(SCOPES[scope], ".knowledge/records/");
                    for (p, re) in record_paths(&h).into_iter().filter(|(p, _)| p.starts_with(&dir)) {
                        let text = String::from_utf8_lossy(&bytes(&h, &re)).replace(&format!("'{}", FILES[file]), &format!("'{rel}"));
                        h.file(&p, text);
                    }
                }
            }
            Edit::MoveFolder { scope, to, rewrite } => {
                let (from, target) = (format!("{}/", SCOPES[scope]), ["svc/c/", "lib/a/"][to]);
                move_prefix(&mut h, &from, target);
                // The root's anchors into the moved folder, rewritten to its new place.
                if rewrite {
                    for (p, re) in record_paths(&h).into_iter().filter(|(p, _)| p.starts_with(".knowledge/")) {
                        let text = String::from_utf8_lossy(&bytes(&h, &re)).replace(&format!("'{from}"), &format!("'{target}"));
                        h.file(&p, text);
                    }
                }
            }
            Edit::Rewrite { pick, how } => {
                let recs = record_paths(&h);
                if recs.is_empty() {
                    continue;
                }
                let (p, re) = &recs[pick % recs.len()];
                let text = String::from_utf8_lossy(&bytes(&h, re)).into_owned();
                let new = match how {
                    0 => text.replacen("title: 't", "title: 'renamed ", 1),
                    1 => text.replacen("must:", "assumes:", 1),
                    2 => text.replacen("'>= 2'", "'>= 4'", 1).replacen("'exist'", "'>= 1'", 1),
                    3 => text.replacen("knowledge: 1", "knowledge: 2", 1),
                    4 => match (text.find("checks:\n"), text.find("---\nWhy")) {
                        (Some(a), Some(z)) => format!("{}{}", &text[..a], &text[z..]),
                        _ => text,
                    },
                    5 => text.replacen("kind: 'decision'", "kind: 'question'", 1),
                    6 => text + "<<<<<<< ours\nx\n=======\ny\n>>>>>>> theirs\n",
                    _ => text.replacen("id: 'k3v9q2x7m00", "id: 'k3v9q2x7m01", 1),
                };
                h.file(p, new);
            }
            Edit::Supersede { pick, scope } => {
                let recs = record_paths(&h);
                if recs.is_empty() {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes(&h, &recs[pick % recs.len()].1)).into_owned();
                let Some(i) = text.find("id: '").map(|i| &text[i + 5..i + 17]) else { continue };
                let sup = format!("---\nknowledge: 1\nid: 'k3v9q2x7m099'\nkind: 'decision'\ntitle: 'sup'\nsupersedes: '{i}'\n---\nWhy.\n");
                h.file(&at(SCOPES[scope], ".knowledge/records/sup.md"), sup);
            }
            Edit::DeleteRecord { pick } => {
                let recs = record_paths(&h);
                if !recs.is_empty() {
                    h.remove(&recs[pick % recs.len()].0);
                }
            }
            Edit::CopyRecord { pick, scope } => {
                let recs = record_paths(&h);
                if recs.is_empty() {
                    continue;
                }
                let content = bytes(&h, &recs[pick % recs.len()].1);
                h.file(&at(SCOPES[scope], ".knowledge/records/copy.md"), content);
            }
            Edit::DirToSymlink { scope, records } => {
                let dir = at(SCOPES[scope], if records { ".knowledge/records" } else { "d" });
                for (p, _) in h.list(&dir).expect("list") {
                    h.remove(&p);
                }
                h.symlink(&dir, "elsewhere");
            }
        }
    }
    h
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2_000, .. ProptestConfig::default() })]
    #[test]
    fn footprint_equals_reference(b in tree_strategy(), edits in edits_strategy()) {
        let h = apply(&b, &edits);
        prop_assert_eq!(gate(&b, &h, &b.changed_paths(&h)).unwrap(), gate_full(&b, &h).unwrap());
    }
}

/// Records every directory listed and every blob read.
struct Counting<'a> {
    inner: &'a MemTree,
    lists: RefCell<Vec<String>>,
    reads: RefCell<Vec<Vec<Oid>>>,
    hints: RefCell<Vec<Vec<Oid>>>,
}

impl<'a> Counting<'a> {
    fn new(inner: &'a MemTree) -> Self {
        Counting { inner, lists: RefCell::default(), reads: RefCell::default(), hints: RefCell::default() }
    }
}

impl Tree for Counting<'_> {
    fn entry(&self, path: &str) -> Result<Option<Entry>, TreeError> {
        self.inner.entry(path)
    }
    fn list(&self, dir: &str) -> Result<Vec<(String, Entry)>, TreeError> {
        self.lists.borrow_mut().push(dir.to_owned());
        self.inner.list(dir)
    }
    fn read_many(&self, oids: &[Oid]) -> Result<Vec<std::sync::Arc<[u8]>>, TreeError> {
        self.reads.borrow_mut().push(oids.to_vec());
        self.inner.read_many(oids)
    }
    fn prefetch(&self, oids: &[Oid]) {
        self.hints.borrow_mut().push(oids.to_vec());
    }
}

#[test]
fn reads_only_above_changed() {
    let key = |n: u8| Rec { id: n, kind: 0, newer: false, supersedes: None, checks: vec![Chk { anchor: 5, must: true, expect: 1 }] };
    let mut b = MemTree::new();
    for (n, scope) in SCOPES.iter().enumerate() {
        b.file(&at(scope, ".knowledge/records/r.md"), record_text(&key(n as u8 + 1)));
        // Distinct bytes per folder, so no two files share an oid.
        b.file(&at(scope, "v.yaml"), format!("replicas: 3\n# {n}\n"));
    }
    let mut h = b.clone();
    h.file("svc/a/v.yaml", "replicas: 1\n");
    let (cb, ch) = (Counting::new(&b), Counting::new(&h));
    let r = gate(&cb, &ch, &b.changed_paths(&h)).unwrap();
    assert_eq!(r.findings.len(), 1, "{r:?}");
    let oid = |t: &MemTree, p: &str| t.entry(p).unwrap().unwrap().oid;
    // Never read: an unaffected check's anchored file, or a record of a folder off the change's way up.
    let never = [oid(&b, "v.yaml"), oid(&b, "svc/b/v.yaml"), oid(&b, "svc/b/.knowledge/records/r.md")];
    for (name, c) in [("B", &cb), ("H", &ch)] {
        let lists = c.lists.borrow();
        assert!(lists.iter().all(|d| !d.starts_with("svc/b")), "{name} listed {lists:?}");
        let (reads, hints) = (c.reads.borrow(), c.hints.borrow());
        assert!(reads.iter().chain(hints.iter()).flatten().all(|o| !never.contains(o)), "{name} read {reads:?}, hinted {hints:?}");
        // Every blob read was hinted first: one hint for the record files and one for the anchored files, so a
        // partial clone fetches in at most two calls.
        assert!(reads.iter().flatten().all(|o| hints.iter().take(2).flatten().any(|h| h == o)), "{name}: read {reads:?}, hinted {hints:?}");
    }
}
