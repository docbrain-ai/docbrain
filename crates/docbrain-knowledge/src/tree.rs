// SPDX-License-Identifier: MIT
//! Trees (spec §5.3 "The two trees", §4.3 "Paths"). Paths name entries of a tree, never files on disk.
//! An entry is a file, a symbolic link or a submodule entry; a directory is not an entry.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use sha2::{Digest, Sha256};

/// An object id: lowercase hex. For a git tree it is git's object name; for a `MemTree` the sha256 of the
/// bytes (a test tree needs identity, not git's hash).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Oid(Box<str>);

impl Oid {
    /// Parses lowercase or uppercase hex; stored lowercase. `None` for anything else or empty.
    pub fn from_hex(hex: &str) -> Option<Oid> {
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(Oid(hex.to_ascii_lowercase().into_boxed_str()))
    }

    /// The sha256 of `bytes`, as an `Oid`.
    pub fn sha256_of(bytes: &[u8]) -> Oid {
        Oid(hex::encode(Sha256::digest(bytes)).into_boxed_str())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The mode of a tree entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Mode {
    File,
    Executable,
    Symlink,
    Submodule,
}

/// One tree entry. `size` is the blob's size in bytes (0 for a submodule), or 0 when the tree cannot know it
/// without reading the blob (git keeps no size in a tree): every bound is judged again on the bytes read.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Entry {
    pub mode: Mode,
    pub oid: Oid,
    pub size: u64,
}

/// A failure to read a tree. A missing path is not an error (`entry` returns `None`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TreeError {
    #[error("object {0} is not in the repository")]
    MissingObject(Oid),
    #[error("`{command}` failed: {stderr}")]
    Git { command: String, stderr: String },
}

/// A tree to evaluate against: path to entry, entries under a directory, and batched blob reads.
pub trait Tree {
    /// The entry at exactly `path`, if any.
    fn entry(&self, path: &str) -> Result<Option<Entry>, TreeError>;
    /// `list("")` is every entry; `list("a/b")` every entry under `a/b/`. Both in byte order of path.
    fn list(&self, dir: &str) -> Result<Vec<(String, Entry)>, TreeError>;
    /// The blobs of `oids`, in argument order. A blob longer than `ANCHORED_READ_MAX + 1` bytes may be cut to
    /// that length: it is over every bound either way.
    fn read_many(&self, oids: &[Oid]) -> Result<Vec<Arc<[u8]>>, TreeError>;
    /// A hint that the blobs of `oids` are about to be read, in this order. A tree whose reads each cost a round
    /// trip (a partial clone's fetch) may bring them close in one; it holds nothing for the caller, and a failure
    /// is reported by the read that needs the blob. Doing nothing is always correct.
    fn prefetch(&self, oids: &[Oid]) {
        let _ = oids;
    }
}

/// An in-memory tree for tests and the conformance corpus.
#[derive(Debug, Clone, Default)]
pub struct MemTree {
    entries: BTreeMap<String, Entry>,
    blobs: HashMap<Oid, Arc<[u8]>>,
}

impl MemTree {
    pub fn new() -> MemTree {
        MemTree::default()
    }

    fn put(&mut self, path: &str, mode: Mode, bytes: &[u8]) -> &mut MemTree {
        let oid = Oid::sha256_of(bytes);
        self.blobs.insert(oid.clone(), Arc::from(bytes));
        self.entries.insert(path.to_owned(), Entry { mode, oid, size: bytes.len() as u64 });
        self
    }

    /// A regular file.
    pub fn file(&mut self, path: &str, bytes: impl AsRef<[u8]>) -> &mut MemTree {
        self.put(path, Mode::File, bytes.as_ref())
    }

    /// An executable file.
    pub fn exec(&mut self, path: &str, bytes: impl AsRef<[u8]>) -> &mut MemTree {
        self.put(path, Mode::Executable, bytes.as_ref())
    }

    /// A symbolic link; its blob is the target text, as in git.
    pub fn symlink(&mut self, path: &str, target: &str) -> &mut MemTree {
        self.put(path, Mode::Symlink, target.as_bytes())
    }

    /// A submodule entry pointing at `commit`. Text that is not hex is hashed, so any text gives a
    /// stable identity.
    pub fn submodule(&mut self, path: &str, commit: &str) -> &mut MemTree {
        let oid = Oid::from_hex(commit).unwrap_or_else(|| Oid::sha256_of(commit.as_bytes()));
        self.entries.insert(path.to_owned(), Entry { mode: Mode::Submodule, oid, size: 0 });
        self
    }

    /// Removes the entry at `path`, if any.
    pub fn remove(&mut self, path: &str) -> &mut MemTree {
        self.entries.remove(path);
        self
    }

    /// Every path added, deleted, modified or changed in mode from `self` (B) to `head` (H).
    pub fn changed_paths(&self, head: &MemTree) -> ChangedPaths {
        let mut out = Vec::new();
        for (p, e) in &self.entries {
            match head.entries.get(p) {
                Some(h) if h.mode == e.mode && h.oid == e.oid => {}
                _ => out.push(p.clone()),
            }
        }
        for p in head.entries.keys() {
            if !self.entries.contains_key(p) {
                out.push(p.clone());
            }
        }
        ChangedPaths::new(out)
    }
}

impl Tree for MemTree {
    fn entry(&self, path: &str) -> Result<Option<Entry>, TreeError> {
        Ok(self.entries.get(path).cloned())
    }

    fn list(&self, dir: &str) -> Result<Vec<(String, Entry)>, TreeError> {
        if dir.is_empty() {
            return Ok(self.entries.iter().map(|(p, e)| (p.clone(), e.clone())).collect());
        }
        let prefix = format!("{dir}/");
        Ok(self
            .entries
            .range(prefix.clone()..)
            .take_while(|(p, _)| p.starts_with(&prefix))
            .map(|(p, e)| (p.clone(), e.clone()))
            .collect())
    }

    fn read_many(&self, oids: &[Oid]) -> Result<Vec<Arc<[u8]>>, TreeError> {
        oids.iter()
            .map(|o| self.blobs.get(o).cloned().ok_or_else(|| TreeError::MissingObject(o.clone())))
            .collect()
    }
}

/// The paths a change touched: added, deleted, modified or changed in mode. Sorted and unique.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChangedPaths(Vec<String>);

impl ChangedPaths {
    /// Sorts and removes duplicates.
    pub fn new(paths: Vec<String>) -> ChangedPaths {
        let set: BTreeSet<String> = paths.into_iter().collect();
        ChangedPaths(set.into_iter().collect())
    }

    pub fn paths(&self) -> &[String] {
        &self.0
    }

    pub fn contains(&self, path: &str) -> bool {
        self.0.binary_search_by(|p| p.as_str().cmp(path)).is_ok()
    }
}

/// What a blobless clone shows before a fetch, for tests: every entry reports size 0, as git cannot know the
/// size of a blob it does not hold, and the blobs of `inflate` read as `ANCHORED_READ_MAX + 1` bytes.
#[cfg(test)]
pub(crate) struct Unsized<'a> {
    pub(crate) inner: &'a MemTree,
    pub(crate) inflate: Vec<Oid>,
}

#[cfg(test)]
impl Tree for Unsized<'_> {
    fn entry(&self, path: &str) -> Result<Option<Entry>, TreeError> {
        Ok(self.inner.entry(path)?.map(|e| Entry { size: 0, ..e }))
    }
    fn list(&self, dir: &str) -> Result<Vec<(String, Entry)>, TreeError> {
        Ok(self.inner.list(dir)?.into_iter().map(|(p, e)| (p, Entry { size: 0, ..e })).collect())
    }
    fn read_many(&self, oids: &[Oid]) -> Result<Vec<Arc<[u8]>>, TreeError> {
        let mut out = self.inner.read_many(oids)?;
        for (o, b) in oids.iter().zip(&mut out) {
            if self.inflate.contains(o) {
                *b = vec![b' '; crate::bounds::ANCHORED_READ_MAX as usize + 1].into();
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memtree_lists_under_dir_in_byte_order() {
        let mut t = MemTree::new();
        t.file("a/b.yaml", b"x");
        t.file("a/B.yaml", b"y");
        t.file("ab", b"z");
        let names: Vec<_> = t.list("a").unwrap().into_iter().map(|(p, _)| p).collect();
        assert_eq!(names, ["a/B.yaml", "a/b.yaml"]); // "ab" is not under "a/"
        let all: Vec<_> = t.list("").unwrap().into_iter().map(|(p, _)| p).collect();
        assert_eq!(all, ["a/B.yaml", "a/b.yaml", "ab"]);
        // "a-" sorts between "a" and "a/" in bytes; it is not under "a/".
        t.file("a-x", b"w");
        assert_eq!(t.list("a").unwrap().len(), 2);
    }

    #[test]
    fn changed_paths_cover_add_delete_modify_mode() {
        let mut b = MemTree::new();
        b.file("keep", b"1").file("gone", b"2").file("edit", b"3").file("mode", b"4");
        let mut h = b.clone();
        h.remove("gone");
        h.file("edit", b"3x");
        h.exec("mode", b"4");
        h.file("added", b"5");
        assert_eq!(b.changed_paths(&h).paths(), ["added", "edit", "gone", "mode"]);
        assert!(b.changed_paths(&b).paths().is_empty());
    }

    #[test]
    fn read_many_in_argument_order_and_missing_object() {
        let mut t = MemTree::new();
        t.file("a", b"one").file("b", b"two");
        let (oa, ob) = (t.entry("a").unwrap().unwrap().oid, t.entry("b").unwrap().unwrap().oid);
        let got = t.read_many(&[ob.clone(), oa.clone()]).unwrap();
        assert_eq!(&*got[0], b"two");
        assert_eq!(&*got[1], b"one");
        let missing = Oid::sha256_of(b"nope");
        assert_eq!(t.read_many(std::slice::from_ref(&missing)), Err(TreeError::MissingObject(missing)));
    }

    #[test]
    fn entries_carry_mode_and_size() {
        let mut t = MemTree::new();
        t.symlink("l", "target/file").submodule("vendor/x", "ABCDEF01");
        let l = t.entry("l").unwrap().unwrap();
        assert_eq!((l.mode, l.size), (Mode::Symlink, 11));
        let s = t.entry("vendor/x").unwrap().unwrap();
        assert_eq!((s.mode, s.oid.as_str()), (Mode::Submodule, "abcdef01"));
        assert_eq!(t.entry("vendor").unwrap(), None); // a directory is not an entry
    }

    #[test]
    fn oid_from_hex_rejects_empty_and_non_hex() {
        for bad in ["", "abg1", "ab 1", "0x1f"] {
            assert_eq!(Oid::from_hex(bad), None, "{bad:?}");
        }
        assert_eq!(Oid::from_hex("AbC1").unwrap().as_str(), "abc1");
    }

    #[test]
    fn changed_paths_sorted_unique() {
        let c = ChangedPaths::new(vec!["b".into(), "a".into(), "b".into()]);
        assert_eq!(c.paths(), ["a", "b"]);
        assert!(c.contains("a") && !c.contains("c"));
    }
}
