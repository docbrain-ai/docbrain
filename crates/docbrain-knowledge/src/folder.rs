// SPDX-License-Identifier: MIT
//! Folders, record files, supersession and `status` (`SEMANTICS.md` §States, §Routing).

use std::collections::{BTreeMap, BTreeSet};

use crate::bounds::{RECORD_FILE_MAX, RECORDS_PER_PASS};
use crate::check::{CheckState, RecordState, eval_check, worst};
use crate::reason::{Bound, Invalid};
use crate::record::{Id, Kind, Parsed, parse_record};
use crate::resolve::FileCache;
use crate::tree::{Entry, Mode, Oid, Tree, TreeError};

/// Why a `supersedes` has no effect (`SEMANTICS.md` §States).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SupersedeProblem {
    NotInFolder,
    Itself,
    Duplicated,
}

/// One record file of a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordFile {
    pub path: String,
    pub oid: Oid,
    pub parsed: Parsed,
    pub superseded_by: Option<Id>,
}

/// One `.knowledge/` folder: its scope (the directory that holds it, `""` at the root) and its records in byte
/// order of path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub scope: String,
    pub records: Vec<RecordFile>,
    /// Entries under `records/` that are not records, reported once.
    pub ignored: Vec<String>,
    /// Ids held by two or more records, with their paths in byte order.
    pub duplicates: Vec<(Id, Vec<String>)>,
    /// The record path whose `supersedes` has no effect, and why.
    pub supersede_problems: Vec<(String, SupersedeProblem)>,
    pub cycles: Vec<Vec<Id>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordStatus {
    pub path: String,
    pub id: Option<Id>,
    pub kind: Option<Kind>,
    pub title: Option<String>,
    pub state: RecordState,
    pub checks: Vec<CheckState>,
    pub folder_wide: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReport {
    pub records: Vec<RecordStatus>,
    pub folders: Vec<Folder>,
    /// Per folder scope, the records past `RECORDS_PER_PASS` that were not read.
    pub over_bound: Vec<(String, usize)>,
}

const RECORDS: &str = ".knowledge/records";

/// The scope of the folder an entry belongs to: the directory before the first `.knowledge/records/` on
/// its path, `""` at the root. `None` when the path lies in no folder's records.
pub fn scope_of(path: &str) -> Option<&str> {
    let mut start = 0;
    loop {
        let rest = path.get(start..)?;
        if rest.strip_prefix(".knowledge/records/").is_some_and(|after| !after.is_empty()) {
            return Some(if start == 0 { "" } else { path.get(..start - 1)? });
        }
        start += rest.find('/')? + 1;
    }
}

pub(crate) fn records_dir(scope: &str) -> String {
    if scope.is_empty() { RECORDS.to_owned() } else { format!("{scope}/{RECORDS}") }
}

/// A record file is a `*.md` entry that is not a submodule; anything else under `records/` is reported once.
pub(crate) fn is_record(path: &str, e: &Entry) -> bool {
    path.ends_with(".md") && e.mode != Mode::Submodule
}

/// The folders on the way up from each path (the path itself included, so a directory names its own folder), in
/// byte order of scope.
pub fn folder_scopes_above(tree: &dyn Tree, paths: &[String]) -> Result<Vec<String>, TreeError> {
    let mut candidates = BTreeSet::new();
    for p in paths {
        candidates.insert("");
        candidates.extend(p.match_indices('/').map(|(i, _)| &p[..i]));
        candidates.insert(p.as_str());
    }
    let mut scopes = Vec::new();
    for c in candidates {
        let dir = records_dir(c);
        // A directory inside another folder's records is that folder's, never a scope of its own.
        if scope_of(&format!("{dir}/x")) == Some(c) && !tree.list(&dir)?.is_empty() {
            scopes.push(c.to_owned());
        }
    }
    Ok(scopes)
}

/// Every folder in the tree, from one listing, in byte order of scope.
pub fn all_folder_scopes(tree: &dyn Tree) -> Result<Vec<String>, TreeError> {
    let all = tree.list("")?;
    let scopes: BTreeSet<&str> = all.iter().filter_map(|(p, _)| scope_of(p)).collect();
    Ok(scopes.into_iter().map(str::to_owned).collect())
}

/// One folder, its records read in one `read_many`. Records past `RECORDS_PER_PASS` in byte order of path are
/// not read; `status` counts them.
pub fn load_folder(tree: &dyn Tree, scope: &str) -> Result<Folder, TreeError> {
    let entries: Vec<(String, Entry)> =
        tree.list(&records_dir(scope))?.into_iter().filter(|(p, _)| scope_of(p) == Some(scope)).collect();
    let over = over_bound(entries.iter(), RECORDS_PER_PASS);
    Ok(build(tree, scope, entries, &over)?.0)
}

/// Whether a record file's blob is read: not a link, and within the record bound (judged from the entry).
fn readable(e: &Entry) -> bool {
    e.mode != Mode::Symlink && e.size <= RECORD_FILE_MAX
}

/// The record paths past `budget`, in byte order of path.
fn over_bound<'a>(entries: impl Iterator<Item = &'a (String, Entry)>, budget: usize) -> BTreeSet<String> {
    let mut paths: Vec<&str> = entries.filter(|(p, e)| is_record(p, e)).map(|(p, _)| p.as_str()).collect();
    paths.sort_unstable();
    paths.into_iter().skip(budget).map(str::to_owned).collect()
}

/// A folder from its entries; the second value counts the records in `over`, which are not read.
fn build(tree: &dyn Tree, scope: &str, entries: Vec<(String, Entry)>, over: &BTreeSet<String>) -> Result<(Folder, usize), TreeError> {
    let (mut ignored, mut skipped) = (Vec::new(), 0);
    let mut slots: Vec<(String, Oid, Option<Parsed>)> = Vec::new();
    let mut unread = Vec::new();
    for (p, e) in entries {
        if !is_record(&p, &e) {
            ignored.push(p);
        } else if over.contains(&p) {
            skipped += 1;
        } else {
            // Judged from the entry, before any blob is read.
            let at = |reason| Some(Parsed::Invalid { reason, line: 1, col: 1 });
            let judged = if e.mode == Mode::Symlink {
                at(Invalid::SymlinkRecord)
            } else if e.size > RECORD_FILE_MAX {
                at(Invalid::Bound(Bound::RecordFile))
            } else {
                unread.push(e.oid.clone());
                None
            };
            slots.push((p, e.oid, judged));
        }
    }
    let mut blobs = if unread.is_empty() { Vec::new() } else { tree.read_many(&unread)? }.into_iter();
    let mut records = Vec::with_capacity(slots.len());
    for (path, oid, judged) in slots {
        let parsed = match judged {
            Some(p) => p,
            None => parse_record(scope, &blobs.next().ok_or_else(|| TreeError::MissingObject(oid.clone()))?),
        };
        records.push(RecordFile { path, oid, parsed, superseded_by: None });
    }
    let mut folder = Folder {
        scope: scope.to_owned(),
        records,
        ignored,
        duplicates: Vec::new(),
        supersede_problems: Vec::new(),
        cycles: Vec::new(),
    };
    supersede(&mut folder);
    Ok((folder, skipped))
}

fn id_of(p: &Parsed) -> Option<Id> {
    match p {
        Parsed::Valid(r) => Some(r.id),
        Parsed::Newer { id, .. } => *id,
        Parsed::Invalid { .. } => None,
    }
}

/// A record's own id and the id it supersedes. The key's meaning is fixed at schema 1, so a newer record's
/// counts (`SEMANTICS.md` §States); a record without an id supersedes nothing.
fn supersedes_of(p: &Parsed) -> Option<(Id, Id)> {
    match p {
        Parsed::Valid(r) => r.supersedes.map(|t| (r.id, t)),
        Parsed::Newer { id: Some(id), supersedes: Some(t), .. } => Some((*id, *t)),
        Parsed::Newer { .. } | Parsed::Invalid { .. } => None,
    }
}

/// `SEMANTICS.md` §States: duplicates, the `supersedes` that have no effect, cycles, and who supersedes whom.
fn supersede(f: &mut Folder) {
    let mut by_id: BTreeMap<Id, Vec<usize>> = BTreeMap::new();
    for (i, r) in f.records.iter().enumerate() {
        if let Some(id) = id_of(&r.parsed) {
            by_id.entry(id).or_default().push(i);
        }
    }
    f.duplicates = by_id
        .iter()
        .filter(|(_, at)| at.len() > 1)
        .map(|(id, at)| (*id, at.iter().map(|&i| f.records[i].path.clone()).collect()))
        .collect();
    // Each record supersedes at most one, so the edges form a functional graph.
    let mut target: Vec<Option<usize>> = vec![None; f.records.len()];
    for (i, r) in f.records.iter().enumerate() {
        let Some((own, t)) = supersedes_of(&r.parsed) else { continue };
        let problem = match by_id.get(&t).map(Vec::as_slice) {
            _ if t == own => SupersedeProblem::Itself,
            None | Some([]) => SupersedeProblem::NotInFolder,
            Some([one]) => {
                target[i] = Some(*one);
                continue;
            }
            Some(_) => SupersedeProblem::Duplicated,
        };
        f.supersede_problems.push((r.path.clone(), problem));
    }
    let in_cycle = cycles(f, &target);
    for (i, t) in target.iter().enumerate() {
        let Some(t) = *t else { continue };
        if !in_cycle[t] && f.records[t].superseded_by.is_none() {
            f.records[t].superseded_by = id_of(&f.records[i].parsed);
        }
    }
}

/// Records each cycle once, starting at its smallest id, and returns which records lie on one.
fn cycles(f: &mut Folder, target: &[Option<usize>]) -> Vec<bool> {
    let n = target.len();
    // 0: not seen; 1: on the walk being followed; 2: done.
    let (mut seen, mut in_cycle) = (vec![0u8; n], vec![false; n]);
    for start in 0..n {
        let (mut walk, mut v) = (Vec::<usize>::new(), Some(start));
        while let Some(x) = v {
            if seen[x] == 1 {
                let from = walk.iter().position(|&w| w == x).unwrap_or(0);
                let mut ids: Vec<Id> = walk[from..].iter().filter_map(|&w| id_of(&f.records[w].parsed)).collect();
                for &w in &walk[from..] {
                    in_cycle[w] = true;
                }
                let least = ids.iter().enumerate().min_by_key(|(_, id)| **id).map_or(0, |(i, _)| i);
                ids.rotate_left(least);
                f.cycles.push(ids);
                break;
            }
            if seen[x] == 2 {
                break;
            }
            seen[x] = 1;
            walk.push(x);
            v = target[x];
        }
        for w in walk {
            seen[w] = 2;
        }
    }
    f.cycles.sort();
    in_cycle
}

/// Every folder's records and their states, in byte order of path.
/// The folders at `scopes`, read within `budget` records (routing reads only these).
pub(crate) fn load_folders(tree: &dyn Tree, scopes: &[String], budget: usize) -> Result<Vec<Folder>, TreeError> {
    Ok(load_folders_counted(tree, scopes, budget)?.0)
}

/// As `load_folders`, with whether `budget` left any record unread.
pub(crate) fn load_folders_counted(tree: &dyn Tree, scopes: &[String], budget: usize) -> Result<(Vec<Folder>, bool), TreeError> {
    let mut groups = BTreeMap::new();
    for s in scopes {
        let entries = tree.list(&records_dir(s))?.into_iter().filter(|(p, _)| scope_of(p) == Some(s.as_str())).collect();
        groups.insert(s.clone(), entries);
    }
    let (folders, counted) = build_within(tree, groups, budget)?;
    Ok((folders, !counted.is_empty()))
}

/// The folders built, and per folder how many records the bound left unread.
type Built = (Vec<Folder>, Vec<(String, usize)>);

/// Each folder of `groups`, the records past `budget` in byte order of path across them all left unread and
/// counted per folder.
fn build_within(tree: &dyn Tree, groups: BTreeMap<String, Vec<(String, Entry)>>, budget: usize) -> Result<Built, TreeError> {
    let over = over_bound(groups.values().flatten(), budget);
    // Every folder's record files, hinted together: a partial clone fetches them in one call.
    let want: Vec<Oid> = groups.values().flatten().filter(|(p, e)| is_record(p, e) && !over.contains(p) && readable(e)).map(|(_, e)| e.oid.clone()).collect();
    tree.prefetch(&want);
    let (mut folders, mut counted) = (Vec::with_capacity(groups.len()), Vec::new());
    for (scope, entries) in groups {
        let (folder, skipped) = build(tree, &scope, entries, &over)?;
        if skipped > 0 {
            counted.push((scope, skipped));
        }
        folders.push(folder);
    }
    Ok((folders, counted))
}

pub fn status(tree: &dyn Tree) -> Result<StatusReport, TreeError> {
    status_within(tree, RECORDS_PER_PASS)
}

fn status_within(tree: &dyn Tree, budget: usize) -> Result<StatusReport, TreeError> {
    let mut groups: BTreeMap<String, Vec<(String, Entry)>> = BTreeMap::new();
    for (p, e) in tree.list("")? {
        if let Some(s) = scope_of(&p) {
            let s = s.to_owned();
            groups.entry(s).or_default().push((p, e));
        }
    }
    let (folders, counted) = build_within(tree, groups, budget)?;
    let mut cache = FileCache::default();
    let mut records = Vec::new();
    for f in &folders {
        for r in &f.records {
            records.push(record_status(tree, r, &mut cache)?);
        }
    }
    records.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(StatusReport { records, folders, over_bound: counted })
}

/// `invalid`, else `superseded` (its checks not evaluated), else the worst of its checks. A newer schema's
/// record is shown and never judged.
pub(crate) fn record_status(tree: &dyn Tree, f: &RecordFile, cache: &mut FileCache) -> Result<RecordStatus, TreeError> {
    let path = f.path.clone();
    let superseded = f.superseded_by.map(|by| RecordState::Superseded { by });
    Ok(match &f.parsed {
        Parsed::Invalid { reason, line, col } => RecordStatus {
            path,
            id: None,
            kind: None,
            title: None,
            state: RecordState::Invalid { reason: *reason, line: *line, col: *col },
            checks: Vec::new(),
            folder_wide: false,
        },
        Parsed::Newer { id, kind, title, applies_to, .. } => RecordStatus {
            path,
            id: *id,
            kind: *kind,
            title: title.clone(),
            state: superseded.unwrap_or(RecordState::Unverifiable),
            checks: Vec::new(),
            folder_wide: applies_to.iter().any(|g| g.rel == "**") && f.superseded_by.is_none(),
        },
        Parsed::Valid(r) => {
            let (state, checks) = match superseded {
                Some(s) => (s, Vec::new()),
                None => {
                    let checks = r.checks.iter().map(|c| eval_check(tree, c, cache)).collect::<Result<Vec<_>, _>>()?;
                    (worst(&checks), checks)
                }
            };
            // `SEMANTICS.md` §States, folder-wide records: context for every file of its folder; a superseded record is context for none.
            let wide = (r.checks.is_empty() && !r.applies_written) || r.applies_to.iter().any(|g| g.rel == "**");
            let folder_wide = wide && f.superseded_by.is_none();
            RecordStatus { path, id: Some(r.id), kind: Some(r.kind), title: Some(r.title.clone()), state, checks, folder_wide }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reason::{Fail, Shown};
    use crate::tree::MemTree;
    use std::cell::RefCell;
    use std::sync::Arc;

    const A: &str = "k3v9q2x7m1aa";
    const B: &str = "k3v9q2x7m1ab";
    const C: &str = "k3v9q2x7m1ac";
    const D: &str = "k3v9q2x7m1ad";
    const E: &str = "k3v9q2x7m1ae";

    fn id(s: &str) -> Id {
        Id::parse(s).unwrap()
    }
    fn rec(id: &str, extra: &str) -> String {
        format!("---\nknowledge: 1\nid: '{id}'\nkind: 'fact'\ntitle: 'T {id}'\n{extra}---\nWhy.\n")
    }
    fn sup(by: &str, target: &str) -> String {
        rec(by, &format!("supersedes: '{target}'\n"))
    }
    fn newer(id: &str, target: &str) -> String {
        format!("---\nknowledge: 2\nid: '{id}'\nkind: 'fact'\ntitle: 'N'\nsupersedes: '{target}'\nwhatever: ['1']\n---\n")
    }
    fn by(f: &Folder, path: &str) -> Option<Id> {
        f.records.iter().find(|r| r.path == path).unwrap_or_else(|| panic!("no {path}")).superseded_by
    }
    fn state<'a>(r: &'a StatusReport, path: &str) -> &'a RecordStatus {
        r.records.iter().find(|s| s.path == path).unwrap_or_else(|| panic!("no {path}"))
    }

    #[test]
    fn scopes() {
        let mut t = MemTree::new();
        t.file(".knowledge/README.md", "x")
            .file(".knowledge/records/a.md", rec(A, ""))
            .file(".knowledge/records/x/.knowledge/records/y.md", rec(B, ""))
            .file("svc/.knowledge/records/2026/b.md", rec(B, ""))
            .file("svc/api/.knowledge/records/c.txt", "x")
            .file("docs/.knowledge/README.md", "x")
            .file("lib/.knowledge/records", "a file, not a directory")
            .file("a-b/.knowledge/records/d.md", rec(D, ""));
        assert_eq!(all_folder_scopes(&t).unwrap(), vec!["", "a-b", "svc", "svc/api"]);
        let above = |p: &[&str]| folder_scopes_above(&t, &p.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        assert_eq!(above(&["svc/api/main.rs"]), vec!["", "svc", "svc/api"]);
        assert_eq!(above(&["svc/api"]), vec!["", "svc", "svc/api"]);
        assert_eq!(above(&["svc/apix/main.rs", "a-b/x"]), vec!["", "a-b", "svc"]);
        assert_eq!(above(&["docs/x.md", "lib/x.rs"]), vec![""]);
        // A path inside a folder's records names that folder only.
        assert_eq!(above(&[".knowledge/records/x/.knowledge/records/y.md"]), vec![""]);
        assert_eq!(above(&[]), Vec::<String>::new());
        // A directory inside a folder's records is not a scope: its entries are that folder's.
        assert!(load_folder(&t, ".knowledge/records/x").unwrap().records.is_empty());
        assert_eq!(load_folder(&t, "").unwrap().records.len(), 2);
        let empty = MemTree::new();
        assert_eq!(all_folder_scopes(&empty).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn record_entries() {
        let mut t = MemTree::new();
        t.file(".knowledge/records/a.md", rec(A, ""))
            .file(".knowledge/records/2026/team/b.md", rec(B, ""))
            .file(".knowledge/records/notes.txt", "x")
            .file(".knowledge/records/A.MD", "x")
            .file(".knowledge/records/run.cmd", "x")
            .symlink(".knowledge/records/link.md", "a.md")
            .submodule(".knowledge/records/sub.md", "4d7a2c1f00000000000000000000000000000000")
            .file(".knowledge/records/big.md", vec![b'x'; RECORD_FILE_MAX as usize + 1])
            .file(".knowledge/records/edge.md", format!("{}{}", rec(C, ""), " ".repeat(RECORD_FILE_MAX as usize - rec(C, "").len())));
        let f = load_folder(&t, "").unwrap();
        let paths: Vec<&str> = f.records.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                ".knowledge/records/2026/team/b.md",
                ".knowledge/records/a.md",
                ".knowledge/records/big.md",
                ".knowledge/records/edge.md",
                ".knowledge/records/link.md",
            ]
        );
        assert_eq!(
            f.ignored,
            vec![".knowledge/records/A.MD", ".knowledge/records/notes.txt", ".knowledge/records/run.cmd", ".knowledge/records/sub.md"]
        );
        let parsed = |p: &str| &f.records.iter().find(|r| r.path == p).unwrap().parsed;
        assert_eq!(parsed(".knowledge/records/link.md"), &Parsed::Invalid { reason: Invalid::SymlinkRecord, line: 1, col: 1 });
        assert_eq!(parsed(".knowledge/records/big.md"), &Parsed::Invalid { reason: Invalid::Bound(Bound::RecordFile), line: 1, col: 1 });
        assert!(matches!(parsed(".knowledge/records/edge.md"), Parsed::Valid(_)));
        assert!(matches!(parsed(".knowledge/records/a.md"), Parsed::Valid(_)));
    }

    /// Counts `read_many` calls and refuses to read any blob over `RECORD_FILE_MAX`.
    struct Reads<'a> {
        inner: &'a MemTree,
        calls: RefCell<Vec<usize>>,
        read: RefCell<Vec<Oid>>,
        hints: RefCell<Vec<Vec<Oid>>>,
    }
    impl Tree for Reads<'_> {
        fn entry(&self, path: &str) -> Result<Option<Entry>, TreeError> {
            self.inner.entry(path)
        }
        fn list(&self, dir: &str) -> Result<Vec<(String, Entry)>, TreeError> {
            self.inner.list(dir)
        }
        fn read_many(&self, oids: &[Oid]) -> Result<Vec<Arc<[u8]>>, TreeError> {
            self.calls.borrow_mut().push(oids.len());
            self.read.borrow_mut().extend_from_slice(oids);
            let blobs = self.inner.read_many(oids)?;
            assert!(blobs.iter().all(|b| b.len() as u64 <= RECORD_FILE_MAX), "read an oversized record");
            Ok(blobs)
        }
        fn prefetch(&self, oids: &[Oid]) {
            self.hints.borrow_mut().push(oids.to_vec());
        }
    }

    #[test]
    fn record_bound_on_the_bytes_read() {
        // A blobless clone reports size 0 for a blob it does not hold: the record bound is judged once read.
        let mut t = MemTree::new();
        t.file(".knowledge/records/big.md", format!("{}{}", rec(A, ""), " ".repeat(RECORD_FILE_MAX as usize)));
        let tree = crate::tree::Unsized { inner: &t, inflate: Vec::new() };
        let f = load_folder(&tree, "").unwrap();
        assert_eq!(f.records[0].parsed, Parsed::Invalid { reason: Invalid::Bound(Bound::RecordFile), line: 1, col: 1 });
    }

    #[test]
    fn one_hint_per_pass_and_none_oversized() {
        let mut t = MemTree::new();
        t.file(".knowledge/records/a.md", rec(A, ""))
            .file(".knowledge/records/b.md", rec(B, ""))
            .file(".knowledge/records/big.md", vec![b'x'; RECORD_FILE_MAX as usize + 1])
            .file("svc/.knowledge/records/c.md", rec(C, ""));
        let r = Reads { inner: &t, calls: RefCell::default(), read: RefCell::default(), hints: RefCell::default() };
        let n = |v: &RefCell<Vec<Vec<Oid>>>| v.borrow().iter().map(Vec::len).collect::<Vec<_>>();
        let hinted_first = || r.read.borrow().iter().all(|o| r.hints.borrow().first().is_some_and(|h| h.contains(o)));
        // One folder: its records in one read.
        load_folder(&r, "").unwrap();
        assert_eq!(*r.calls.borrow(), vec![2]);
        r.read.borrow_mut().clear();
        let report = status(&r).unwrap();
        assert_eq!(report.records.len(), 4);
        // One hint for the records of every folder of the pass, so a partial clone fetches once; each is read
        // after it was hinted, and the oversized one never. No check here reads a file.
        assert_eq!((n(&r.hints), r.read.borrow().len()), (vec![3], 3));
        assert!(hinted_first());
    }

    #[test]
    fn a_code_file_is_read_once_per_pass() {
        let mut t = MemTree::new();
        let retry = "retry(3, backoff::exponential());";
        let code = |f: &str| format!("  - code: '{f}'\n    text: '{retry}'\n    must: 'exist'\n");
        t.file("src/a.rs", format!("{retry}\n"))
            .file("src/b.rs", "nothing here\n")
            .file(".knowledge/records/a.md", rec(A, &format!("checks:\n{}{}", code("src/a.rs"), code("src/b.rs"))))
            .file(".knowledge/records/b.md", rec(B, &format!("checks:\n{}{}", code("src/a.rs"), code("src/b.rs"))));
        let r = Reads { inner: &t, calls: RefCell::default(), read: RefCell::default(), hints: RefCell::default() };
        let report = status(&r).unwrap();
        assert_eq!(state(&report, ".knowledge/records/b.md").state, RecordState::Broken);
        // The records, then each code file once, whichever record asks first.
        assert_eq!(*r.calls.borrow(), vec![2, 1, 1]);
    }

    #[test]
    fn supersession_same_folder_only() {
        let mut t = MemTree::new();
        t.file(".knowledge/records/a.md", rec(A, ""))
            .file("svc/.knowledge/records/b.md", rec(B, ""))
            .file("svc/.knowledge/records/c.md", sup(C, A))
            .file("svc/.knowledge/records/d.md", sup(D, D))
            .file("svc/.knowledge/records/e.md", sup(E, B))
            .file("svc/.knowledge/records/f.md", sup("k3v9q2x7m1af", "k3v9q2x7m1zz"));
        let root = load_folder(&t, "").unwrap();
        assert_eq!(by(&root, ".knowledge/records/a.md"), None);
        let svc = load_folder(&t, "svc").unwrap();
        assert_eq!(by(&svc, "svc/.knowledge/records/b.md"), Some(id(E)));
        for p in ["c", "d", "e", "f"] {
            assert_eq!(by(&svc, &format!("svc/.knowledge/records/{p}.md")), None, "{p}");
        }
        assert_eq!(
            svc.supersede_problems,
            vec![
                ("svc/.knowledge/records/c.md".to_owned(), SupersedeProblem::NotInFolder),
                ("svc/.knowledge/records/d.md".to_owned(), SupersedeProblem::Itself),
                ("svc/.knowledge/records/f.md".to_owned(), SupersedeProblem::NotInFolder),
            ]
        );
        // A superseded record is not evaluated; its superseder is.
        let report = status(&t).unwrap();
        assert_eq!(state(&report, "svc/.knowledge/records/b.md").state, RecordState::Superseded { by: id(E) });
        assert!(state(&report, "svc/.knowledge/records/b.md").checks.is_empty());
        assert_eq!(state(&report, "svc/.knowledge/records/e.md").state, RecordState::Holds);
        assert_eq!(state(&report, ".knowledge/records/a.md").state, RecordState::Holds);
    }

    #[test]
    fn chains_and_several_superseders() {
        let mut t = MemTree::new();
        // e supersedes d, d supersedes c: both c and d are superseded.
        t.file(".knowledge/records/c.md", rec(C, ""))
            .file(".knowledge/records/d.md", sup(D, C))
            .file(".knowledge/records/e.md", sup(E, D))
            // Two records supersede a: the first by path names it.
            .file(".knowledge/records/a.md", rec(A, ""))
            .file(".knowledge/records/x2.md", sup("k3v9q2x7m1b2", A))
            .file(".knowledge/records/x1.md", sup("k3v9q2x7m1b1", A));
        let f = load_folder(&t, "").unwrap();
        assert_eq!(by(&f, ".knowledge/records/c.md"), Some(id(D)));
        assert_eq!(by(&f, ".knowledge/records/d.md"), Some(id(E)));
        assert_eq!(by(&f, ".knowledge/records/e.md"), None);
        assert_eq!(by(&f, ".knowledge/records/a.md"), Some(id("k3v9q2x7m1b1")));
        assert!(f.supersede_problems.is_empty() && f.cycles.is_empty());
    }

    #[test]
    fn cycle_supersedes_nobody() {
        let mut t = MemTree::new();
        t.file(".knowledge/records/a.md", sup(A, B))
            .file(".knowledge/records/b.md", sup(B, A))
            // c points into the cycle: a stays unsuperseded.
            .file(".knowledge/records/c.md", sup(C, A))
            // A three-cycle met first in path order, though its ids sort after the first; e superseded by nobody.
            .file(".knowledge/records/0.md", sup("k3v9q2x7m1b1", "k3v9q2x7m1b2"))
            .file(".knowledge/records/1.md", sup("k3v9q2x7m1b2", "k3v9q2x7m1b3"))
            .file(".knowledge/records/2.md", sup("k3v9q2x7m1b3", "k3v9q2x7m1b1"))
            .file(".knowledge/records/e.md", sup(E, C));
        let f = load_folder(&t, "").unwrap();
        for p in ["a", "b", "0", "1", "2", "e"] {
            assert_eq!(by(&f, &format!(".knowledge/records/{p}.md")), None, "{p}");
        }
        assert_eq!(by(&f, ".knowledge/records/c.md"), Some(id(E)));
        assert_eq!(
            f.cycles,
            vec![vec![id(A), id(B)], vec![id("k3v9q2x7m1b1"), id("k3v9q2x7m1b2"), id("k3v9q2x7m1b3")]]
        );
        assert!(f.supersede_problems.is_empty());
    }

    #[test]
    fn newer_schema_supersedes_honoured() {
        let mut t = MemTree::new();
        t.file(".knowledge/records/a.md", rec(A, ""))
            .file(".knowledge/records/n.md", newer(B, A))
            .file(".knowledge/records/c.md", sup(C, B));
        let f = load_folder(&t, "").unwrap();
        let n = &f.records.iter().find(|r| r.path.ends_with("n.md")).unwrap().parsed;
        assert!(matches!(n, Parsed::Newer { .. }), "{n:?}");
        assert_eq!(by(&f, ".knowledge/records/a.md"), Some(id(B)));
        assert_eq!(by(&f, ".knowledge/records/n.md"), Some(id(C)));
        let report = status(&t).unwrap();
        let n = state(&report, ".knowledge/records/n.md");
        assert_eq!(n.state, RecordState::Superseded { by: id(C) });
        assert_eq!((n.id, n.title.as_deref()), (Some(id(B)), Some("N")));
        // Not superseded, a newer record is unverifiable, shown, never judged.
        t.remove(".knowledge/records/c.md");
        let report = status(&t).unwrap();
        let n = state(&report, ".knowledge/records/n.md");
        assert_eq!((&n.state, n.checks.len(), n.kind), (&RecordState::Unverifiable, 0, Some(Kind::Fact)));
    }

    #[test]
    fn duplicates_keep_evaluating() {
        let mut t = MemTree::new();
        t.file("a.yaml", "x: 1\n")
            .file(".knowledge/records/a1.md", rec(A, "checks:\n  - config_key: 'a.yaml#x'\n    must: '== 1'\n"))
            .file(".knowledge/records/a2.md", rec(A, "checks:\n  - config_key: 'a.yaml#x'\n    must: '== 2'\n"))
            .file(".knowledge/records/n.md", format!("---\nknowledge: 2\nid: '{A}'\n---\n"))
            .file(".knowledge/records/b.md", sup(B, A))
            .file(".knowledge/records/bad.md", "no front matter\n");
        let f = load_folder(&t, "").unwrap();
        assert_eq!(
            f.duplicates,
            vec![(id(A), vec![".knowledge/records/a1.md".to_owned(), ".knowledge/records/a2.md".into(), ".knowledge/records/n.md".into()])]
        );
        assert_eq!(f.supersede_problems, vec![(".knowledge/records/b.md".to_owned(), SupersedeProblem::Duplicated)]);
        assert!(f.records.iter().all(|r| r.superseded_by.is_none()));
        let report = status(&t).unwrap();
        assert_eq!(state(&report, ".knowledge/records/a1.md").state, RecordState::Holds);
        assert_eq!(state(&report, ".knowledge/records/a2.md").state, RecordState::Broken);
        assert_eq!(
            state(&report, ".knowledge/records/a2.md").checks,
            vec![CheckState::Broken(Fail::False { actual: Shown::new("1") })]
        );
        assert!(matches!(state(&report, ".knowledge/records/bad.md").state, RecordState::Invalid { .. }));
        assert_eq!(state(&report, ".knowledge/records/bad.md").id, None);
    }

    #[test]
    fn record_state_from_checks() {
        let mut t = MemTree::new();
        let checks = "checks:\n  - path: 'a.yaml'\n    must: 'exist'\n  - config_key: 'a.yaml#y'\n    assumes: '== 1'\n  - config_key: 'a.yaml#x'\n    must: '> 1'\n";
        t.file("svc/a.yaml", "x: .inf\n").file("svc/.knowledge/records/a.md", rec(A, checks));
        let report = status(&t).unwrap();
        let s = state(&report, "svc/.knowledge/records/a.md");
        assert_eq!(
            s.checks,
            vec![
                CheckState::Holds(None),
                CheckState::Stale(Fail::MissingKey),
                CheckState::Unverifiable(crate::reason::Unverifiable::NotComparable),
            ]
        );
        assert_eq!(s.state, RecordState::Stale);
        assert_eq!((s.id, s.kind, s.title.as_deref()), (Some(id(A)), Some(Kind::Fact), Some("T k3v9q2x7m1aa")));
    }

    #[test]
    fn folder_wide_flag() {
        let mut t = MemTree::new();
        t.file("svc/.knowledge/records/a.md", rec(A, ""))
            .file("svc/.knowledge/records/b.md", rec(B, "applies_to: ['**']\n"))
            .file("svc/.knowledge/records/c.md", rec(C, "applies_to: ['src/**']\n"))
            .file("svc/.knowledge/records/d.md", rec(D, "checks:\n  - path: 'src'\n    must: 'exist'\n"))
            .file("svc/.knowledge/records/e.md", rec(E, "applies_to: ['src/**', '**']\n"))
            .file("svc/.knowledge/records/f.md", "---\nknowledge: 2\nid: 'k3v9q2x7m1af'\n---\n")
            .file("svc/.knowledge/records/g.md", "nope\n")
            // A superseded record is context for nothing; its superseder, with no checks, for everything.
            .file("svc/.knowledge/records/h.md", rec("k3v9q2x7m1ah", ""))
            .file("svc/.knowledge/records/j.md", sup("k3v9q2x7m1aj", "k3v9q2x7m1ah"))
            // A newer record routes by its written applies_to, so `**` there is folder-wide too.
            .file("svc/.knowledge/records/k.md", "---\nknowledge: 2\nid: 'k3v9q2x7m1ak'\napplies_to: ['**']\n---\n")
            .file("svc/.knowledge/records/m.md", "---\nknowledge: 2\nid: 'k3v9q2x7m1am'\napplies_to: ['**']\n---\n")
            .file("svc/.knowledge/records/n.md", sup("k3v9q2x7m1an", "k3v9q2x7m1am"));
        let report = status(&t).unwrap();
        let wide: Vec<(char, bool)> = report
            .records
            .iter()
            .map(|r| (r.path.chars().nth_back(3).unwrap(), r.folder_wide))
            .collect();
        assert_eq!(
            wide,
            vec![
                ('a', true),
                ('b', true),
                ('c', false),
                ('d', false),
                ('e', true),
                ('f', false),
                ('g', false),
                ('h', false),
                ('j', true),
                ('k', true),
                ('m', false),
                ('n', true)
            ]
        );
    }

    #[test]
    fn records_per_pass_bound() {
        let mut t = MemTree::new();
        for i in 0..RECORDS_PER_PASS + 3 {
            t.file(&format!(".knowledge/records/{i:06}.md"), "x");
        }
        let report = status(&t).unwrap();
        assert_eq!(report.over_bound, vec![(String::new(), 3)]);
        assert_eq!(report.records.len(), RECORDS_PER_PASS);
        assert_eq!(report.records[0].path, ".knowledge/records/000000.md");
        assert_eq!(report.records[RECORDS_PER_PASS - 1].path, format!(".knowledge/records/{:06}.md", RECORDS_PER_PASS - 1));
        assert_eq!(report.folders[0].records.len(), RECORDS_PER_PASS);
    }

    #[test]
    fn bound_follows_path_order_across_folders() {
        // `a-b/` sorts before `a/` by byte ('-' < '/'), though the scope `a` sorts before `a-b`.
        let mut t = MemTree::new();
        t.file("a/.knowledge/records/1.md", rec(A, ""))
            .file("a/.knowledge/records/2.md", rec(B, ""))
            .file("a-b/.knowledge/records/1.md", rec(C, ""))
            .file("z/.knowledge/records/1.md", rec(D, ""));
        let report = status_within(&t, 2).unwrap();
        let paths: Vec<&str> = report.records.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, vec!["a-b/.knowledge/records/1.md", "a/.knowledge/records/1.md"]);
        assert_eq!(report.over_bound, vec![("a".to_owned(), 1), ("z".to_owned(), 1)]);
        let scopes: Vec<&str> = report.folders.iter().map(|f| f.scope.as_str()).collect();
        assert_eq!(scopes, vec!["a", "a-b", "z"]);
        // A full budget reads everything and counts nothing.
        let report = status_within(&t, 4).unwrap();
        assert_eq!((report.records.len(), report.over_bound.len()), (4, 0));
    }
}
