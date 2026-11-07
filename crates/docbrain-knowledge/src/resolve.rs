// SPDX-License-Identifier: MIT
//! Resolving a `config_key` (spec §4.3 "`config_key`", §7.6; `SEMANTICS.md` §Resolving values): layers, globs,
//! documents and key paths, over a tree.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::bounds::{FILES_HELD_MAX, anchored_file_max};
use crate::format::{Format, HclBody, format_of, merge_files, read_hcl, read_json, read_toml, read_yaml_file};
use crate::reason::{Bound, Fail, Unverifiable};
use crate::record::{Layer, Op, Seg};
use crate::scalar::{Scalar, compare};
use crate::tree::{Mode, Oid, Tree, TreeError};
use crate::value::Value;

const MISSING: Resolved = Resolved::Missing(Fail::MissingKey);
const LFS_POINTER: &[u8] = b"version https://git-lfs.github.com/spec/";
/// C21: a Git LFS pointer is a blob under this many bytes.
const LFS_POINTER_MAX: usize = 1_024;

/// What a `config_key` resolves to. `Present` holds whatever is at the key, a map or list included; a scalar
/// whose reading is unknown is still present (`Scalar::Opaque`), so `exist` holds and a comparison says why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    Present(Value),
    Missing(Fail),
    Unverifiable(Unverifiable),
}

/// What one pass keeps of the anchored files it has read (`SEMANTICS.md` §Bounds): parsed documents, HCL
/// bodies, merged HCL sets and squeezed `code` text, each charged the memory it holds (an estimate: a parsed
/// file can hold many times its bytes), plus `ENTRY` and its key, so a reason (an unreadable or oversized
/// file) is charged as well, within a budget (`FILES_HELD_MAX`). The least recently used is let go first and read again when asked for, so what the cache
/// holds changes how long a pass takes, never an answer.
pub struct FileCache {
    budget: u64,
    held: u64,
    tick: u64,
    kept: HashMap<Key, (Kept, u64, u64)>,
    by_use: BTreeMap<u64, Key>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Docs(Oid, Format),
    Body(Oid),
    Merged(Vec<Oid>),
    Code(Oid),
}

#[derive(Clone)]
enum Kept {
    Docs(Arc<Result<Vec<Value>, Unverifiable>>),
    /// With the bytes it was read from: a glob's HCL files are bounded together on those.
    Body(Arc<Result<HclBody, Unverifiable>>, u64),
    Merged(Arc<Value>),
    Code(Result<Arc<str>, Unverifiable>),
}

#[cfg(test)]
thread_local! {
    /// A budget for every `FileCache::default()` on this thread: tests run the corpus as a pass that keeps nothing.
    pub(crate) static BUDGET: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    /// Files let go on this thread, kept or never kept: a test asserts its budget was exercised.
    pub(crate) static LET_GO: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Default for FileCache {
    fn default() -> FileCache {
        #[cfg(test)]
        if let Some(b) = BUDGET.with(std::cell::Cell::get) {
            return FileCache::with_budget(b);
        }
        FileCache::with_budget(FILES_HELD_MAX)
    }
}

impl FileCache {
    pub(crate) fn with_budget(budget: u64) -> FileCache {
        FileCache { budget, held: 0, tick: 0, kept: HashMap::new(), by_use: BTreeMap::new() }
    }

    /// The bytes held now.
    #[cfg(test)]
    pub(crate) fn held(&self) -> u64 {
        self.held
    }

    fn holds(&self, k: &Key) -> bool {
        self.kept.contains_key(k)
    }

    /// What is kept under `k`, now the most recently used.
    fn get(&mut self, k: &Key) -> Option<Kept> {
        let (v, _, at) = self.kept.get_mut(k)?;
        self.tick += 1;
        self.by_use.remove(at);
        *at = self.tick;
        self.by_use.insert(self.tick, k.clone());
        Some(v.clone())
    }

    /// Keeps `v`, which holds `heap` bytes, letting the least recently used go until it fits; what can never fit
    /// is not kept.
    fn put(&mut self, k: Key, v: Kept, heap: usize) {
        let cost = bytes(heap).saturating_add(ENTRY).saturating_add(k.bytes());
        if let Some((_, c, at)) = self.kept.remove(&k) {
            self.held = self.held.saturating_sub(c);
            self.by_use.remove(&at);
        }
        if cost > self.budget {
            let_go();
            return;
        }
        while self.held.saturating_add(cost) > self.budget {
            let Some((_, old)) = self.by_use.pop_first() else { break };
            if let Some((_, c, _)) = self.kept.remove(&old) {
                self.held = self.held.saturating_sub(c);
                let_go();
            }
        }
        self.tick += 1;
        self.held = self.held.saturating_add(cost);
        self.by_use.insert(self.tick, k.clone());
        self.kept.insert(k, (v, cost, self.tick));
    }

    pub(crate) fn code(&mut self, oid: &Oid) -> Option<Result<Arc<str>, Unverifiable>> {
        match self.get(&Key::Code(oid.clone())) {
            Some(Kept::Code(h)) => Some(h),
            _ => None,
        }
    }

    /// The squeezed text of a `code` file.
    pub(crate) fn put_code(&mut self, oid: Oid, h: Result<Arc<str>, Unverifiable>) {
        let heap = h.as_ref().map_or(0, |t| t.len());
        self.put(Key::Code(oid), Kept::Code(h), heap);
    }
}

/// What keeping anything costs beyond the bytes it was read from and its key: the maps' entries and the
/// value's own allocation, a generous round figure.
const ENTRY: u64 = 256;

impl Key {
    /// The bytes of the object names in the key.
    fn bytes(&self) -> u64 {
        match self {
            Key::Docs(o, _) | Key::Body(o) | Key::Code(o) => bytes(o.as_str().len()),
            Key::Merged(os) => os.iter().fold(0u64, |n, o| n.saturating_add(bytes(o.as_str().len()))),
        }
    }
}

fn let_go() {
    #[cfg(test)]
    LET_GO.with(|n| n.set(n.get() + 1));
}

fn bytes(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Layers are read last to first; the first that defines the key wins. A layer defines the key when it holds a
/// value there or cannot be read (it may hold one).
pub fn resolve(tree: &dyn Tree, layers: &[Layer], cache: &mut FileCache) -> Result<Resolved, TreeError> {
    let mut any_file = false;
    for l in layers.iter().rev() {
        match layer(tree, l, cache)? {
            None => {}
            Some(Resolved::Missing(_)) => any_file = true,
            Some(r) => return Ok(r),
        }
    }
    Ok(Resolved::Missing(if any_file { Fail::MissingKey } else { Fail::MissingFile }))
}

/// The directory a path lies in, `""` at the root.
pub(crate) fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// C14: the reason a link or a submodule gives when it is `dir` or any directory above it.
pub(crate) fn link_above(tree: &dyn Tree, dir: &str) -> Result<Option<Unverifiable>, TreeError> {
    for (end, _) in dir.match_indices('/').chain([(dir.len(), "")]) {
        if end > 0
            && let Some(u) = tree.entry(&dir[..end])?.and_then(|e| link_reason(e.mode))
        {
            return Ok(Some(u));
        }
    }
    Ok(None)
}

pub(crate) fn link_reason(mode: Mode) -> Option<Unverifiable> {
    match mode {
        Mode::Symlink => Some(Unverifiable::SymbolicLink),
        Mode::Submodule => Some(Unverifiable::Submodule),
        Mode::File | Mode::Executable => None,
    }
}

/// One layer; `None` when no file matches.
fn layer(tree: &dyn Tree, l: &Layer, cache: &mut FileCache) -> Result<Option<Resolved>, TreeError> {
    let glob = &l.file;
    // C14: a link or a submodule as any directory above the file, or above a glob's matches.
    let base = if glob.is_literal() { parent(&glob.root) } else { glob.literal_dir() };
    if let Some(u) = link_above(tree, base)? {
        return Ok(Some(Resolved::Unverifiable(u)));
    }
    let matches = if glob.is_literal() {
        tree.entry(&glob.root)?.into_iter().map(|e| (glob.root.clone(), e)).collect()
    } else {
        tree.list(glob.literal_dir())?.into_iter().filter(|(p, _)| glob.matches(p)).collect::<Vec<_>>()
    };
    if matches.is_empty() {
        return Ok(None);
    }
    // Judged from the entries, before any blob is read.
    if let Some(u) = matches.iter().find_map(|(_, e)| link_reason(e.mode)) {
        return Ok(Some(Resolved::Unverifiable(u)));
    }
    // Only the files a resolver reads count: one that is never read is `format` whatever its size. A glob's
    // HCL files merge into one value, so together they are bounded as one HCL file.
    let hcl_size = matches.iter().filter(|(p, _)| format_of(p) == Some(Format::Hcl)).fold(0u64, |n, (_, e)| n.saturating_add(e.size));
    if hcl_size > anchored_file_max(Format::Hcl) || matches.iter().any(|(p, e)| format_of(p).is_some_and(|f| e.size > anchored_file_max(f))) {
        return Ok(Some(Resolved::Unverifiable(OVER)));
    }
    let files: Vec<(Option<Format>, Oid, u64)> = matches.into_iter().map(|(p, e)| (format_of(&p), e.oid, e.size)).collect();

    // A partial clone fetches what this layer reads in one call; each file is then read by itself.
    let held = |f: &Option<Format>, oid: &Oid| match f {
        Some(Format::Hcl) => cache.holds(&Key::Body(oid.clone())),
        Some(f) => cache.holds(&Key::Docs(oid.clone(), *f)),
        None => true,
    };
    let unheld: Vec<Oid> = files.iter().filter(|(f, oid, _)| !held(f, oid)).map(|(_, oid, _)| oid.clone()).collect();
    if !unheld.is_empty() {
        tree.prefetch(&unheld);
    }
    // Each file alone, one at a time in path order, except that a glob's HCL files merge into one value (C12),
    // which stands where the first HCL file does, so a reason is the first in path order.
    let segs = &l.key.segs;
    let hcl: Vec<Oid> = files.iter().filter(|(f, ..)| *f == Some(Format::Hcl)).map(|(_, oid, _)| oid.clone()).collect();
    let (mut merged, mut over) = match hcl.is_empty() {
        true => (None, false),
        false => {
            let (r, over) = hcl_files(tree, hcl, segs, cache)?;
            (Some(r), over)
        }
    };
    let mut one = OneOf::default();
    for (format, oid, _) in &files {
        match format {
            None => one.push(Resolved::Unverifiable(Unverifiable::Format)),
            Some(Format::Hcl) => one.extend(merged.take()),
            Some(f) => {
                let d = docs(tree, *f, oid, cache)?;
                over |= matches!(*d, Err(OVER));
                one.push(match &*d {
                    Ok(d) => documents(d, segs),
                    Err(u) => Resolved::Unverifiable(*u),
                });
            }
        }
    }
    // Again on the bytes read, before any file's own reason: a partial clone reports size 0 for a blob it does
    // not hold.
    Ok(Some(if over { Resolved::Unverifiable(OVER) } else { one.finish(Unverifiable::DefinedTwice) }))
}

const OVER: Unverifiable = Unverifiable::Bound(Bound::AnchoredFile);

/// One blob, read by itself.
fn one_blob(tree: &dyn Tree, oid: &Oid) -> Result<Arc<[u8]>, TreeError> {
    tree.read_many(std::slice::from_ref(oid))?.into_iter().next().ok_or_else(|| TreeError::MissingObject(oid.clone()))
}

/// The documents of `oid` read in format `f`: kept, else read and parsed.
fn docs(tree: &dyn Tree, f: Format, oid: &Oid, cache: &mut FileCache) -> Result<Arc<Result<Vec<Value>, Unverifiable>>, TreeError> {
    let key = Key::Docs(oid.clone(), f);
    if let Some(Kept::Docs(d)) = cache.get(&key) {
        return Ok(d);
    }
    let blob = one_blob(tree, oid)?;
    let parse = |src: &str| match f {
        Format::Json => read_json(src).map(|v| vec![v]),
        Format::Toml => read_toml(src).map(|v| vec![v]),
        _ => read_yaml_file(src),
    };
    let d = Arc::new(bounded(&blob, f).and_then(parse));
    let heap = d.as_ref().as_ref().map_or(0, |d| d.iter().fold(d.capacity().saturating_mul(std::mem::size_of::<Value>()), |n, v| n.saturating_add(v.heap())));
    cache.put(key, Kept::Docs(d.clone()), heap);
    Ok(d)
}

/// `text`, unless the blob is over its format's anchored-file bound.
fn bounded(b: &[u8], f: Format) -> Result<&str, Unverifiable> {
    if bytes(b.len()) > anchored_file_max(f) { Err(OVER) } else { text(b) }
}

/// C21 and §8 item 62: an LFS pointer, then UTF-8.
pub(crate) fn text(bytes: &[u8]) -> Result<&str, Unverifiable> {
    if bytes.len() < LFS_POINTER_MAX && bytes.starts_with(LFS_POINTER) {
        return Err(Unverifiable::LfsPointer);
    }
    std::str::from_utf8(bytes).map_err(|_| Unverifiable::Encoding)
}

/// A glob's HCL files, in path order, as one merged value, and whether they were over the bound once read:
/// together they are bounded as one HCL file, so reading stops at the file that takes them past it. A file that
/// cannot be read gives its reason (the first).
fn hcl_files(tree: &dyn Tree, oids: Vec<Oid>, segs: &[Seg], cache: &mut FileCache) -> Result<(Resolved, bool), TreeError> {
    let key = Key::Merged(oids.clone());
    if let Some(Kept::Merged(v)) = cache.get(&key) {
        return Ok((walk(&v, segs), false));
    }
    let (mut bodies, mut size, mut reason) = (Vec::with_capacity(oids.len()), 0u64, None);
    for oid in &oids {
        let (b, n) = match cache.get(&Key::Body(oid.clone())) {
            Some(Kept::Body(b, n)) => (b, n),
            _ => {
                let blob = one_blob(tree, oid)?;
                let b = Arc::new(bounded(&blob, Format::Hcl).and_then(read_hcl));
                let n = if b.is_ok() { bytes(blob.len()) } else { 0 };
                let heap = b.as_ref().as_ref().map_or(0, HclBody::heap);
                cache.put(Key::Body(oid.clone()), Kept::Body(b.clone(), n), heap);
                (b, n)
            }
        };
        size = size.saturating_add(n);
        if matches!(*b, Err(OVER)) || size > anchored_file_max(Format::Hcl) {
            return Ok((Resolved::Unverifiable(OVER), true));
        }
        match &*b {
            Ok(body) => bodies.push(body.clone()),
            Err(u) => {
                reason.get_or_insert(*u);
            }
        }
    }
    if let Some(u) = reason {
        return Ok((Resolved::Unverifiable(u), false));
    }
    let merged = merge_files(&bodies);
    let r = walk(&merged, segs);
    let heap = merged.heap();
    cache.put(key, Kept::Merged(Arc::new(merged)), heap);
    Ok((r, false))
}

/// The key in exactly one of several places, taken one place at a time: two values are `two`; otherwise a
/// place that cannot be read may hold it, so its reason (the first) wins over one value. Only the first value
/// is kept.
#[derive(Default)]
struct OneOf {
    found: Option<Value>,
    count: usize,
    reason: Option<Unverifiable>,
}

impl OneOf {
    fn push(&mut self, o: Resolved) {
        match o {
            Resolved::Present(v) => {
                self.count += 1;
                self.found.get_or_insert(v);
            }
            Resolved::Unverifiable(u) => {
                self.reason.get_or_insert(u);
            }
            Resolved::Missing(_) => {}
        }
    }

    fn finish(self, two: Unverifiable) -> Resolved {
        match (self.count, self.reason, self.found) {
            (2.., _, _) => Resolved::Unverifiable(two),
            (_, Some(u), _) => Resolved::Unverifiable(u),
            (_, None, Some(v)) => Resolved::Present(v),
            _ => MISSING,
        }
    }
}

impl Extend<Resolved> for OneOf {
    fn extend<I: IntoIterator<Item = Resolved>>(&mut self, outcomes: I) {
        outcomes.into_iter().for_each(|o| self.push(o));
    }
}

fn one_of(outcomes: impl IntoIterator<Item = Resolved>, two: Unverifiable) -> Resolved {
    let mut one = OneOf::default();
    one.extend(outcomes);
    one.finish(two)
}

/// C11: one document is that document; with two or more, a leading index or selector picks among them, and a
/// key must be in exactly one.
fn documents(docs: &[Value], segs: &[Seg]) -> Resolved {
    match (docs, segs.split_first()) {
        ([], _) => MISSING,
        ([one], _) => walk(one, segs),
        (_, Some((Seg::Index(n), rest))) => docs.get(*n).map_or(MISSING, |d| walk(d, rest)),
        (_, Some((Seg::Select(criteria), rest))) => match select(docs, criteria) {
            Ok(d) => walk(d, rest),
            Err(r) => r,
        },
        _ => one_of(docs.iter().map(|d| walk(d, segs)), Unverifiable::Ambiguous),
    }
}

fn is_template(s: &str) -> bool {
    s.contains("{{") || s.contains("{%")
}

fn walk(mut v: &Value, segs: &[Seg]) -> Resolved {
    for seg in segs {
        let next = match seg {
            Seg::Key(k) => key(v, k),
            Seg::Index(n) => elements(v).and_then(|items| items.get(*n).ok_or(MISSING)),
            Seg::Select(criteria) => elements(v).and_then(|items| select(items, criteria)),
        };
        match next {
            Ok(x) => v = x,
            Err(r) => return r,
        }
    }
    // §7.6: a value nobody can read is unverifiable whatever the expectation, `exist` included. A number no
    // decimal holds (`.inf`) is read, so it is a value that no op compares.
    match (opaque(v), v) {
        (Some(u), _) => Resolved::Unverifiable(u),
        (None, &Value::Scalar(Scalar::Opaque(u))) if u != Unverifiable::NotComparable => Resolved::Unverifiable(u),
        _ => Resolved::Present(v.clone()),
    }
}

/// What going into `v` cannot pass: a value nobody can read, or a template that may render to a map or a list.
fn opaque(v: &Value) -> Option<Unverifiable> {
    match v {
        Value::Opaque(u) => Some(*u),
        Value::Scalar(Scalar::Str(s)) if is_template(s) => Some(Unverifiable::Template),
        _ => None,
    }
}

/// One key step; `Err` is the answer. A non-empty list needs a selector (C12).
fn key<'a>(v: &'a Value, k: &str) -> Result<&'a Value, Resolved> {
    if let Some(u) = opaque(v) {
        return Err(Resolved::Unverifiable(u));
    }
    match v {
        Value::Map { entries, merge_key } => match entries.iter().find(|(e, _)| e == k) {
            Some((_, x)) => Ok(x),
            None if *merge_key => Err(Resolved::Unverifiable(Unverifiable::MergeKey)),
            None => Err(MISSING),
        },
        Value::List(items) if !items.is_empty() => Err(Resolved::Unverifiable(Unverifiable::Ambiguous)),
        _ => Err(MISSING),
    }
}

/// What an index or a selector picks from: a list, or a map as the one element it is (a block or a document
/// that occurs once, C11 and C12).
fn elements(v: &Value) -> Result<&[Value], Resolved> {
    if let Some(u) = opaque(v) {
        return Err(Resolved::Unverifiable(u));
    }
    match v {
        Value::List(items) => Ok(items),
        Value::Map { .. } => Ok(std::slice::from_ref(v)),
        _ => Err(MISSING),
    }
}

/// C16: the one element whose keys equal the values, as `==` compares. Two certain matches are ambiguous; an
/// element whose comparison is unverifiable may match, so its reason wins over one match.
fn select<'a>(items: &'a [Value], criteria: &[(Vec<String>, String)]) -> Result<&'a Value, Resolved> {
    let (mut found, mut count, mut reason) = (None, 0, None);
    for item in items {
        match matches(item, criteria) {
            Ok(true) => {
                count += 1;
                found.get_or_insert(item);
            }
            Ok(false) => {}
            Err(u) => {
                reason.get_or_insert(u);
            }
        }
    }
    match (count, reason, found) {
        (2.., _, _) => Err(Resolved::Unverifiable(Unverifiable::Ambiguous)),
        (_, Some(u), _) => Err(Resolved::Unverifiable(u)),
        (_, None, Some(item)) => Ok(item),
        _ => Err(MISSING),
    }
}

/// Every criterion holds; one that fails decides, whatever the others' reasons.
fn matches(item: &Value, criteria: &[(Vec<String>, String)]) -> Result<bool, Unverifiable> {
    let mut reason = None;
    for (path, literal) in criteria {
        match criterion(item, path, literal) {
            Ok(true) => {}
            Ok(false) => return Ok(false),
            Err(u) => {
                reason.get_or_insert(u);
            }
        }
    }
    reason.map_or(Ok(true), Err)
}

fn criterion(item: &Value, path: &[String], literal: &str) -> Result<bool, Unverifiable> {
    let mut v = item;
    for k in path {
        v = match key(v, k) {
            Ok(x) => x,
            Err(Resolved::Unverifiable(u)) => return Err(u),
            Err(_) => return Ok(false),
        };
    }
    if let Some(u) = opaque(v) {
        return Err(u);
    }
    match v {
        Value::Scalar(s) => compare(s, Op::Eq, literal),
        // A map or a list never equals a literal.
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;
    use std::time::Instant;

    use super::*;
    use crate::bounds::{CODE_FILE_MAX, anchored_file_max};
    use crate::path::parse_rel;
    use crate::reason::Bound;
    use crate::record::parse_key_path;
    use crate::scalar::{Decimal, Scalar};
    use crate::tree::{Entry, MemTree, Oid};

    fn layers(anchors: &[&str]) -> Vec<Layer> {
        anchors
            .iter()
            .map(|a| {
                let (f, k) = a.split_once('#').unwrap();
                Layer { file: parse_rel("", f).unwrap(), key: parse_key_path(k).unwrap() }
            })
            .collect()
    }
    fn res(t: &dyn Tree, anchors: &[&str]) -> Resolved {
        resolve(t, &layers(anchors), &mut FileCache::default()).unwrap()
    }
    fn st(s: &str) -> Resolved {
        Resolved::Present(Value::Scalar(Scalar::Str(s.into())))
    }
    fn num(s: &str) -> Resolved {
        Resolved::Present(Value::Scalar(Scalar::Number(Decimal::parse_number(s).unwrap())))
    }
    fn unv(u: Unverifiable) -> Resolved {
        Resolved::Unverifiable(u)
    }
    const MISSING_KEY: Resolved = Resolved::Missing(Fail::MissingKey);
    const MISSING_FILE: Resolved = Resolved::Missing(Fail::MissingFile);

    #[test]
    fn helm_layers_last_wins() {
        let ls = ["deploy/base/values.yaml#resources.limits.memory", "deploy/prod/values.yaml#resources.limits.memory"];
        let mut t = MemTree::new();
        t.file("deploy/base/values.yaml", "resources:\n  limits:\n    memory: 256Mi\n");
        t.file("deploy/prod/values.yaml", "resources:\n  limits:\n    memory: 512Mi\n");
        assert_eq!(res(&t, &ls), st("512Mi"));
        // A layer that does not define the key defines nothing; a missing file neither.
        t.file("deploy/prod/values.yaml", "replicas: 3\n");
        assert_eq!(res(&t, &ls), st("256Mi"));
        t.remove("deploy/prod/values.yaml");
        assert_eq!(res(&t, &ls), st("256Mi"));
        // Missing everywhere: the key, unless no layer's file exists at all.
        t.file("deploy/base/values.yaml", "replicas: 3\n");
        assert_eq!(res(&t, &ls), MISSING_KEY);
        t.remove("deploy/base/values.yaml");
        assert_eq!(res(&t, &ls), MISSING_FILE);
        // A later layer that cannot be read may define the key: its reason wins over an earlier value.
        t.file("deploy/base/values.yaml", "resources:\n  limits:\n    memory: 256Mi\n");
        t.file("deploy/prod/values.yaml", "resources: [\n");
        assert_eq!(res(&t, &ls), unv(Unverifiable::Format));
        // An earlier layer that cannot be read is overridden by a later value.
        t.file("deploy/base/values.yaml", "resources: [\n");
        t.file("deploy/prod/values.yaml", "resources:\n  limits:\n    memory: 512Mi\n");
        assert_eq!(res(&t, &ls), st("512Mi"));
    }

    #[test]
    fn glob_merges_and_defined_twice() {
        let mut t = MemTree::new();
        t.file("infra/prod/main.tf", "resource \"aws_db_instance\" \"prod\" {\n  multi_az = true\n}\n");
        t.file("infra/prod/versions.tf", "terraform {\n  required_version = \">= 1.5\"\n}\n");
        t.file("infra/prod/backend.tf", "terraform {\n  backend \"s3\" {\n    bucket = \"b\"\n  }\n}\n");
        t.file("infra/prod/sub/x.tf", "terraform {\n  required_version = \"1\"\n}\n");
        t.file("infra/prod/README.md", "# notes\n");
        let multi_az = Resolved::Present(Value::Scalar(Scalar::Bool(true)));
        assert_eq!(res(&t, &["infra/prod/*.tf#resource.aws_db_instance.prod.multi_az"]), multi_az);
        // `*` stays in its directory, and blocks merge across files.
        assert_eq!(res(&t, &["infra/prod/*.tf#terraform.required_version"]), st(">= 1.5"));
        assert_eq!(res(&t, &["infra/prod/*.tf#terraform.backend.s3.bucket"]), st("b"));
        t.file("infra/prod/override.tf", "terraform {\n  required_version = \">= 1.6\"\n}\n");
        assert_eq!(res(&t, &["infra/prod/*.tf#terraform.required_version"]), unv(Unverifiable::DefinedTwice));
        // A glob matching a file no resolver reads: that file may hold the key.
        assert_eq!(res(&t, &["infra/prod/*#resource.aws_db_instance.prod.multi_az"]), unv(Unverifiable::Format));
        // No match: the layer defines nothing.
        assert_eq!(res(&t, &["infra/stage/*.tf#terraform.required_version"]), MISSING_FILE);
        // YAML: each file is read alone; a key path in two files is defined twice.
        t.file("config/a.yaml", "db:\n  host: x\n");
        t.file("config/b.yaml", "db:\n  port: 1\n");
        assert_eq!(res(&t, &["config/*.yaml#db.port"]), num("1"));
        assert_eq!(res(&t, &["config/*.yaml#db.host"]), st("x"));
        assert_eq!(res(&t, &["config/*.yaml#db.user"]), MISSING_KEY);
        t.file("config/c.yaml", "db:\n  port: 2\n");
        assert_eq!(res(&t, &["config/*.yaml#db.port"]), unv(Unverifiable::DefinedTwice));
        // One value and one file that cannot be read: the reason, since that file may hold the key too.
        t.remove("config/c.yaml");
        t.file("config/c.yaml", "db: [\n");
        assert_eq!(res(&t, &["config/*.yaml#db.port"]), unv(Unverifiable::Format));
    }

    #[test]
    fn multi_document_rules() {
        let mut t = MemTree::new();
        t.file(
            "apps/api/jobs.yaml",
            "kind: Job\nmetadata:\n  name: api-migrate\n  annotations:\n    argocd.argoproj.io/sync-wave: '-1'\nspec: x\n---\nkind: Deployment\nmetadata:\n  name: api\n",
        );
        let j = "apps/api/jobs.yaml#";
        assert_eq!(res(&t, &[&format!("{j}metadata.name")]), unv(Unverifiable::Ambiguous));
        let wave = format!("{j}[kind=Job,metadata.name=api-migrate].metadata.annotations.\"argocd.argoproj.io/sync-wave\"");
        assert_eq!(res(&t, &[&wave]), st("-1"));
        assert_eq!(res(&t, &[&format!("{j}[1].kind")]), st("Deployment"));
        assert_eq!(res(&t, &[&format!("{j}[2].kind")]), MISSING_KEY);
        assert_eq!(res(&t, &[&format!("{j}[kind=Service].kind")]), MISSING_KEY);
        // A key in exactly one document.
        assert_eq!(res(&t, &[&format!("{j}spec")]), st("x"));
        assert_eq!(res(&t, &[&format!("{j}nothing")]), MISSING_KEY);
        // Two documents match the selector.
        t.file("apps/api/jobs.yaml", "kind: Job\nn: 1\n---\nkind: Job\nn: 2\n");
        assert_eq!(res(&t, &[&format!("{j}[kind=Job].n")]), unv(Unverifiable::Ambiguous));
        // Two documents match it and only one has the key: still two.
        t.file("apps/api/jobs.yaml", "kind: Job\nn: 1\n---\nkind: Job\n");
        assert_eq!(res(&t, &[&format!("{j}[kind=Job].n")]), unv(Unverifiable::Ambiguous));
        // One document that is a list: an index or a selector picks among its elements.
        t.file("apps/api/list.json", "[{\"k\": 1}, {\"k\": 2}]");
        assert_eq!(res(&t, &["apps/api/list.json#[1].k"]), num("2"));
        assert_eq!(res(&t, &["apps/api/list.json#[k=2].k"]), num("2"));
        // One document is that document, and a selector or index still applies to it.
        t.file("apps/api/jobs.yaml", "kind: Job\nmetadata:\n  name: api-migrate\n");
        assert_eq!(res(&t, &[&format!("{j}[kind=Job,metadata.name=api-migrate].metadata.name")]), st("api-migrate"));
        assert_eq!(res(&t, &[&format!("{j}[0].kind")]), st("Job"));
        assert_eq!(res(&t, &[&format!("{j}[kind=Deployment].kind")]), MISSING_KEY);
        assert_eq!(res(&t, &[&format!("{j}metadata.name")]), st("api-migrate"));
        // An empty file defines nothing.
        t.file("apps/api/jobs.yaml", "");
        assert_eq!(res(&t, &[&format!("{j}kind")]), MISSING_KEY);
    }

    #[test]
    fn selector_numbers_compare_as_eq() {
        let mut t = MemTree::new();
        t.file(
            "infra/sg.tf",
            "resource \"aws_security_group\" \"web\" {\n  ingress {\n    from_port = 443\n    to_port = 443\n  }\n  ingress {\n    from_port = 80\n    to_port = 8080\n  }\n}\n",
        );
        let web = "infra/*.tf#resource.aws_security_group.web.";
        assert_eq!(res(&t, &[&format!("{web}ingress[from_port=443].to_port")]), num("443"));
        assert_eq!(res(&t, &[&format!("{web}ingress[from_port=80].to_port")]), num("8080"));
        assert_eq!(res(&t, &[&format!("{web}ingress[from_port=22].to_port")]), MISSING_KEY);
        // A repeated nested block needs a selector (C12).
        assert_eq!(res(&t, &[&format!("{web}ingress.to_port")]), unv(Unverifiable::Ambiguous));
        // A block that occurs once is its body, and a selector still applies.
        t.file("infra/sg.tf", "resource \"aws_security_group\" \"web\" {\n  ingress {\n    from_port = 443\n    to_port = 443\n  }\n}\n");
        assert_eq!(res(&t, &[&format!("{web}ingress[from_port=443].to_port")]), num("443"));
        assert_eq!(res(&t, &[&format!("{web}ingress.to_port")]), num("443"));
        // YAML: the comparison is `==`'s, so the text "443" equals 443 as well; an element whose
        // comparison is unverifiable makes the selector unverifiable.
        t.file("p.yaml", "ports:\n- {port: 443, x: 1}\n- {port: '80', x: 2}\n- {port: on, x: 3}\n");
        assert_eq!(res(&t, &["p.yaml#ports[port=80].x"]), unv(Unverifiable::Yaml11));
        t.file("p.yaml", "ports:\n- {port: 443, x: 1}\n- {port: '80', x: 2}\n- {x: 3}\n- 7\n");
        assert_eq!(res(&t, &["p.yaml#ports[port=443].x"]), num("1"));
        assert_eq!(res(&t, &["p.yaml#ports[port=80].x"]), num("2"));
        // Two certain matches are ambiguous even beside an unverifiable element.
        t.file("p.yaml", "ports:\n- {port: 1, x: 1}\n- {port: 1, x: 2}\n- {port: on}\n");
        assert_eq!(res(&t, &["p.yaml#ports[port=1].x"]), unv(Unverifiable::Ambiguous));
        // A selector value that is a map never equals a literal.
        t.file("p.yaml", "ports:\n- {port: {a: 1}, x: 1}\n- {port: 2, x: 2}\n");
        assert_eq!(res(&t, &["p.yaml#ports[port=2].x"]), num("2"));
        assert_eq!(res(&t, &["p.yaml#ports[port=1].x"]), MISSING_KEY);
        // Index out of range, a key on a scalar, an index on a scalar, a key on an empty list.
        t.file("p.yaml", "l: [a]\ns: 5\ne: []\n");
        assert_eq!(res(&t, &["p.yaml#l[1]"]), MISSING_KEY);
        assert_eq!(res(&t, &["p.yaml#l[0]"]), st("a"));
        assert_eq!(res(&t, &["p.yaml#s.x"]), MISSING_KEY);
        assert_eq!(res(&t, &["p.yaml#s[0]"]), MISSING_KEY);
        assert_eq!(res(&t, &["p.yaml#e.x"]), MISSING_KEY);
        assert_eq!(res(&t, &["p.yaml#l.x"]), unv(Unverifiable::Ambiguous));
    }

    #[test]
    fn cargo_lock_package_selector() {
        let mut t = MemTree::new();
        t.file("Cargo.lock", "version = 4\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"sqlx\"\nversion = \"0.8.6\"\n");
        assert_eq!(res(&t, &["Cargo.lock#package[name=sqlx].version"]), st("0.8.6"));
        assert_eq!(res(&t, &["Cargo.lock#package[name=tokio].version"]), MISSING_KEY);
        assert_eq!(res(&t, &["Cargo.lock#version"]), num("4"));
    }

    #[test]
    fn merge_key_absent_key_unverifiable() {
        let mut t = MemTree::new();
        t.file("c.yaml", "base: &b\n  a: 1\nsvc:\n  <<: *b\n  c: 2\n  l:\n  - {<<: *b, k: x}\n");
        assert_eq!(res(&t, &["c.yaml#svc.c"]), num("2"));
        assert_eq!(res(&t, &["c.yaml#svc.a"]), unv(Unverifiable::MergeKey));
        assert_eq!(res(&t, &["c.yaml#svc.zzz"]), unv(Unverifiable::MergeKey));
        assert_eq!(res(&t, &["c.yaml#base.zzz"]), MISSING_KEY);
        // A selector key absent from a merge-key element is unverifiable too.
        assert_eq!(res(&t, &["c.yaml#svc.l[a=1].k"]), unv(Unverifiable::MergeKey));
    }

    #[test]
    fn alias_expanded_value_resolves() {
        let mut t = MemTree::new();
        t.file("compose.yaml", "x-common: &c\n  image: 'a:1'\nsvc: *c\nsvc2:\n  <<: *c\n  port: 80\n");
        assert_eq!(res(&t, &["compose.yaml#svc.image"]), st("a:1"));
        assert_eq!(res(&t, &["compose.yaml#svc2.port"]), num("80"));
        assert_eq!(res(&t, &["compose.yaml#svc2.image"]), unv(Unverifiable::MergeKey));
    }

    #[test]
    fn ansible_template_value_only() {
        let mut t = MemTree::new();
        t.file("group_vars/all.yml", "db_host: \"{{ lookup('env', 'DB') }}\"\ndb_port: 5432\nmode: '{% if a %}x{% endif %}'\nhosts: \"{{ groups.web }}\"\n");
        // §7.6: unverifiable whatever the expectation, `exist` included.
        let tmpl = unv(Unverifiable::Template);
        assert_eq!(res(&t, &["group_vars/all.yml#db_host"]), tmpl);
        assert_eq!(res(&t, &["group_vars/all.yml#mode"]), tmpl);
        assert_eq!(res(&t, &["group_vars/all.yml#db_port"]), num("5432"));
        // A template may render to a map or a list: going into one is unverifiable.
        assert_eq!(res(&t, &["group_vars/all.yml#hosts[0]"]), unv(Unverifiable::Template));
        assert_eq!(res(&t, &["group_vars/all.yml#db_host.x"]), unv(Unverifiable::Template));
        // In every format, and inside a selector.
        t.file("c.json", "{\"a\": \"{{x}}\", \"l\": [{\"n\": \"{{y}}\", \"v\": 1}]}");
        assert_eq!(res(&t, &["c.json#a"]), tmpl);
        assert_eq!(res(&t, &["c.json#l[n=1].v"]), unv(Unverifiable::Template));
        // `{` alone is no template.
        t.file("c.json", "{\"a\": \"{x}\"}");
        assert_eq!(res(&t, &["c.json#a"]), st("{x}"));
    }

    #[test]
    fn gha_on_key() {
        let mut t = MemTree::new();
        t.file(".github/workflows/ci.yml", "on:\n  push:\n    branches: [main]\nv: on\n");
        let branches = Resolved::Present(Value::List(vec![Value::Scalar(Scalar::Str("main".into()))]));
        assert_eq!(res(&t, &[".github/workflows/ci.yml#on.push.branches"]), branches);
        // §7.6: unverifiable whatever the expectation, `exist` included; still a scalar, so nothing is below it.
        assert_eq!(res(&t, &[".github/workflows/ci.yml#v"]), unv(Unverifiable::Yaml11));
        assert_eq!(res(&t, &[".github/workflows/ci.yml#v.x"]), MISSING_KEY);
        // A number no decimal holds is read, in both versions: a value, which no op compares (§Values and comparison).
        t.file("f.yaml", "i: -.inf\nn: .NaN\n");
        for k in ["i", "n"] {
            assert_eq!(res(&t, &[&format!("f.yaml#{k}")]), Resolved::Present(Value::Scalar(Scalar::Opaque(Unverifiable::NotComparable))));
        }
        // A map holding such a value is a value.
        assert!(matches!(res(&t, &[".github/workflows/ci.yml#on"]), Resolved::Present(Value::Map { .. })));
    }

    #[test]
    fn map_value_is_present_for_exist_not_for_compare() {
        let mut t = MemTree::new();
        t.file("values.yaml", "resources:\n  limits:\n    memory: 1Gi\n");
        match res(&t, &["values.yaml#resources.limits"]) {
            Resolved::Present(Value::Map { entries, .. }) => assert_eq!(entries.len(), 1),
            other => panic!("{other:?}"),
        }
        // A value nobody can read from the file (defined twice, an HCL expression, a tag) is not present.
        t.file("values.yaml", "a: 1\na: 2\nt: !Ref x\n");
        assert_eq!(res(&t, &["values.yaml#a"]), unv(Unverifiable::DefinedTwice));
        assert_eq!(res(&t, &["values.yaml#t"]), unv(Unverifiable::Tag));
        assert_eq!(res(&t, &["values.yaml#t.x"]), unv(Unverifiable::Tag));
        t.file("main.tf", "a = var.x\n");
        assert_eq!(res(&t, &["main.tf#a"]), unv(Unverifiable::Expression));
    }

    #[test]
    fn lfs_symlink_submodule_encoding() {
        let mut t = MemTree::new();
        let pointer = "version https://git-lfs.github.com/spec/v1\noid sha256:4d7a\nsize 12345\n";
        t.file("lfs.yaml", pointer);
        assert_eq!(res(&t, &["lfs.yaml#a"]), unv(Unverifiable::LfsPointer));
        // 1,024 bytes or more is not a pointer, whatever it starts with: it is read, here as a YAML string.
        let padded = |n: usize| format!("{pointer}#{}\n", "x".repeat(n - pointer.len() - 2));
        t.file("lfs.yaml", padded(1023));
        assert_eq!(res(&t, &["lfs.yaml#a"]), unv(Unverifiable::LfsPointer));
        t.file("lfs.yaml", padded(1024));
        assert_eq!(res(&t, &["lfs.yaml#a"]), MISSING_KEY);
        // Links and submodules, as the file, as a directory above it, and as a glob match.
        t.file("real.yaml", "a: 1\n").symlink("link.yaml", "real.yaml");
        assert_eq!(res(&t, &["link.yaml#a"]), unv(Unverifiable::SymbolicLink));
        t.symlink("deploy", "elsewhere");
        assert_eq!(res(&t, &["deploy/values.yaml#a"]), unv(Unverifiable::SymbolicLink));
        t.submodule("vendor/lib", "4d7a2c1f00000000000000000000000000000000");
        assert_eq!(res(&t, &["vendor/lib/values.yaml#a"]), unv(Unverifiable::Submodule));
        assert_eq!(res(&t, &["vendor/lib/*.yaml#a"]), unv(Unverifiable::Submodule));
        assert_eq!(res(&t, &["vendor/lib#a"]), unv(Unverifiable::Submodule));
        t.file("g/a.yaml", "a: 1\n").symlink("g/b.yaml", "a.yaml");
        assert_eq!(res(&t, &["g/*.yaml#a"]), unv(Unverifiable::SymbolicLink));
        // Not UTF-8.
        t.file("bad.yaml", b"a: \xff\n");
        assert_eq!(res(&t, &["bad.yaml#a"]), unv(Unverifiable::Encoding));
        // An executable file is a regular file.
        t.exec("run.yaml", "a: 1\n");
        assert_eq!(res(&t, &["run.yaml#a"]), num("1"));
        // A name no resolver reads.
        t.file("x.ini", "a=1\n");
        assert_eq!(res(&t, &["x.ini#a"]), unv(Unverifiable::Format));
    }

    /// A tree whose entries under `big/` report a size over their bound and under `edge/` the bound itself
    /// (a file's format's, else a `code` check's), and that counts blob reads.
    struct Counting {
        inner: MemTree,
        reads: Cell<usize>,
    }
    impl Counting {
        fn grow(&self, (p, mut e): (String, Entry)) -> (String, Entry) {
            let bound = format_of(&p).map_or(CODE_FILE_MAX, anchored_file_max);
            if p.starts_with("big/") {
                e.size = bound + 1;
            } else if p.starts_with("edge/") {
                e.size = bound;
            }
            (p, e)
        }
    }
    impl Tree for Counting {
        fn entry(&self, path: &str) -> Result<Option<Entry>, TreeError> {
            Ok(self.inner.entry(path)?.map(|e| self.grow((path.to_owned(), e)).1))
        }
        fn list(&self, dir: &str) -> Result<Vec<(String, Entry)>, TreeError> {
            Ok(self.inner.list(dir)?.into_iter().map(|pe| self.grow(pe)).collect())
        }
        fn read_many(&self, oids: &[Oid]) -> Result<Vec<Arc<[u8]>>, TreeError> {
            self.reads.set(self.reads.get() + oids.len());
            self.inner.read_many(oids)
        }
    }

    #[test]
    fn format_before_size_for_files_never_read() {
        // Rule 2 counts only files a resolver reads: a file no resolver reads is `format` whatever its size.
        let mut inner = MemTree::new();
        inner.file("big/x.ini", "a=1\n").file("big/v.yaml", "a: 1\n").file("big/w.ini", "a=1\n");
        let t = Counting { inner, reads: Cell::new(0) };
        assert_eq!(res(&t, &["big/x.ini#a"]), unv(Unverifiable::Format));
        assert_eq!(res(&t, &["big/*.ini#a"]), unv(Unverifiable::Format));
        assert_eq!(res(&t, &["big/*#a"]), unv(Unverifiable::Bound(Bound::AnchoredFile)));
        assert_eq!(t.reads.get(), 0);
    }

    #[test]
    fn bound_on_the_bytes_read() {
        // A blobless clone reports size 0 for a blob it does not hold: the layer's bound is judged again once
        // its files are read, before any file's own reason.
        let mut t = MemTree::new();
        t.file("big/values.yaml", "a: 1\n").file("big/a.tf", "x = 1\n").file("mix/a.yaml", "a: [\n").file("mix/b.yaml", "b: 1\n");
        let inflate = ["big/values.yaml", "big/a.tf", "mix/b.yaml"].iter().map(|p| t.entry(p).unwrap().unwrap().oid).collect();
        let tree = crate::tree::Unsized { inner: &t, inflate };
        let bound = unv(Unverifiable::Bound(Bound::AnchoredFile));
        assert_eq!(res(&tree, &["big/values.yaml#a"]), bound);
        assert_eq!(res(&tree, &["big/*.yaml#a"]), bound);
        assert_eq!(res(&tree, &["big/a.tf#x"]), bound);
        // mix/a.yaml alone does not parse; the layer is still the bound, as in a full clone.
        assert_eq!(res(&tree, &["mix/*.yaml#a"]), bound);
    }

    /// Every entry reports `.1` as its size; blobs read as written.
    struct Sized<'a>(&'a MemTree, u64);
    impl Tree for Sized<'_> {
        fn entry(&self, path: &str) -> Result<Option<Entry>, TreeError> {
            Ok(self.0.entry(path)?.map(|e| Entry { size: self.1, ..e }))
        }
        fn list(&self, dir: &str) -> Result<Vec<(String, Entry)>, TreeError> {
            Ok(self.0.list(dir)?.into_iter().map(|(p, e)| (p, Entry { size: self.1, ..e })).collect())
        }
        fn read_many(&self, oids: &[Oid]) -> Result<Vec<Arc<[u8]>>, TreeError> {
            self.0.read_many(oids)
        }
    }

    #[test]
    fn each_format_has_its_own_bound() {
        // §7.5: per format, above real lockfiles. A 40 MiB package-lock is read; a 20 MiB TOML file is over.
        let mib = 1_048_576;
        let over = unv(Unverifiable::Bound(Bound::AnchoredFile));
        let mut t = MemTree::new();
        t.file("a.json", "{\"a\": 1}").file("a.toml", "a = 1\n").file("a.yaml", "a: 1\n").file("a.tf", "a = 1\n");
        assert_eq!(res(&Sized(&t, 40 * mib), &["a.json#a"]), num("1"));
        assert_eq!(res(&Sized(&t, 20 * mib), &["a.toml#a"]), over);
        for (k, f) in [("a.json#a", Format::Json), ("a.toml#a", Format::Toml), ("a.yaml#a", Format::Yaml), ("a.tf#a", Format::Hcl)] {
            assert_ne!(res(&Sized(&t, anchored_file_max(f)), &[k]), over, "{k} at its bound");
            assert_eq!(res(&Sized(&t, anchored_file_max(f) + 1), &[k]), over, "{k} past its bound");
        }
        // Again on the bytes read: a TOML blob one byte past its bound, under every other bound.
        let pad = usize::try_from(anchored_file_max(Format::Toml)).unwrap() - 5;
        let mut t = MemTree::new();
        t.file("b.toml", format!("a = 1\n{}", " ".repeat(pad)));
        assert_eq!(res(&crate::tree::Unsized { inner: &t, inflate: Vec::new() }, &["b.toml#a"]), over);
        // Exactly at its bound once read: read.
        t.file("b.toml", format!("a = 1\n{}", " ".repeat(pad - 1)));
        assert_eq!(res(&crate::tree::Unsized { inner: &t, inflate: Vec::new() }, &["b.toml#a"]), num("1"));
    }

    #[test]
    fn bound_from_entry_size_before_read() {
        let mut inner = MemTree::new();
        inner.file("big/values.yaml", "a: 1\n").file("big/b.yaml", "a: 1\n").file("small.yaml", "a: 1\n");
        let t = Counting { inner, reads: Cell::new(0) };
        assert_eq!(res(&t, &["big/values.yaml#a"]), unv(Unverifiable::Bound(Bound::AnchoredFile)));
        assert_eq!(res(&t, &["big/*.yaml#a"]), unv(Unverifiable::Bound(Bound::AnchoredFile)));
        assert_eq!(t.reads.get(), 0);
        // One read per pass for a file used by many checks.
        let mut cache = FileCache::default();
        for _ in 0..3 {
            assert_eq!(resolve(&t, &layers(&["small.yaml#a"]), &mut cache).unwrap(), num("1"));
        }
        assert_eq!(t.reads.get(), 1);
        // The bound itself is read.
        let mut inner = MemTree::new();
        inner.file("edge/a.yaml", "a: 1\n").file("docs/README", "a: 1\n").file("docs/a.yaml", "a: 1\n");
        let t = Counting { inner, reads: Cell::new(0) };
        assert_eq!(res(&t, &["edge/a.yaml#a"]), num("1"));
        // A file no resolver reads is never read.
        assert_eq!(res(&t, &["docs/*#a"]), unv(Unverifiable::Format));
        assert_eq!(t.reads.get(), 2);
        // HCL files too are read once per pass, and merged once.
        let mut inner = MemTree::new();
        inner.file("tf/a.tf", "x = 1\n").file("tf/b.tf", "y = 2\n");
        let t = Counting { inner, reads: Cell::new(0) };
        let mut cache = FileCache::default();
        for k in ["x", "y", "x"] {
            assert!(matches!(resolve(&t, &layers(&[&format!("tf/*.tf#{k}")]), &mut cache).unwrap(), Resolved::Present(_)));
        }
        assert!(matches!(resolve(&t, &layers(&["tf/a.tf#x"]), &mut cache).unwrap(), Resolved::Present(_)));
        assert_eq!(t.reads.get(), 2);
    }

    #[test]
    fn bom_crlf_anchored_formats_resolve() {
        let mut t = MemTree::new();
        t.file("a.yaml", "\u{feff}a:\r\n  b: 1\r\n");
        t.file("a.json", "\u{feff}{\"a\": {\"b\": 1}}\r\n");
        t.file("a.toml", "\u{feff}[a]\r\nb = 1\r\n");
        t.file("a.tf", "\u{feff}a {\r\n  b = 1\r\n}\r\n");
        for f in ["a.yaml", "a.json", "a.toml", "a.tf"] {
            assert_eq!(res(&t, &[&format!("{f}#a.b")]), num("1"), "{f}");
        }
        t.file("b.yaml", "\u{feff}\u{feff}a: 1\n");
        assert_eq!(res(&t, &["b.yaml#a"]), unv(Unverifiable::Format));
    }

    #[test]
    fn unicode_and_quoted_segments_resolve() {
        let mut t = MemTree::new();
        t.file("v.yaml", "my key:\n  名前: 太郎\n'a\"b': 1\nmetadata:\n  annotations:\n    argocd.argoproj.io/sync-wave: '5'\n");
        assert_eq!(res(&t, &["v.yaml#\"my key\".名前"]), st("太郎"));
        assert_eq!(res(&t, &["v.yaml#\"a\\\"b\""]), num("1"));
        assert_eq!(res(&t, &["v.yaml#metadata.annotations.\"argocd.argoproj.io/sync-wave\""]), st("5"));
    }

    #[test]
    fn five_mib_json_resolves_quickly() {
        let mut json = String::from("{\"name\": \"app\", \"lockfileVersion\": 3, \"packages\": {");
        let mut i = 0;
        while json.len() < 5 * 1024 * 1024 {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                "\"node_modules/pkg-{i}\": {{\"version\": \"1.{i}.0\", \"resolved\": \"https://registry.example/pkg-{i}/-/pkg-{i}-1.{i}.0.tgz\", \"integrity\": \"sha512-{}\", \"dev\": true, \"engines\": {{\"node\": \">=18\"}}}}",
                "A".repeat(88)
            ));
            i += 1;
        }
        json.push_str("}}");
        let mut t = MemTree::new();
        t.file("package-lock.json", &json);
        let start = Instant::now();
        let got = res(&t, &["package-lock.json#packages.\"node_modules/pkg-777\".version"]);
        let took = start.elapsed();
        eprintln!("5 MiB JSON ({} packages): resolved in {took:?}", i);
        assert_eq!(got, st("1.777.0"));
        // The bound is for an optimised build; an unoptimised test build gets ten times as long.
        let limit_ms = if cfg!(debug_assertions) { 5_000 } else { 500 };
        assert!(took.as_millis() < limit_ms, "{took:?}");
    }

    #[test]
    fn attack_pass() {
        // One blob under two names is parsed once per format: `a: 1` is YAML, not JSON, and `a = 1` is HCL.
        let mut t = MemTree::new();
        t.file("x.yaml", "a: 1\n").file("x.json", "a: 1\n").file("h.tf", "a = 1\n").file("h.yaml", "a = 1\n");
        let mut cache = FileCache::default();
        let mut with = |a: &str| resolve(&t, &layers(&[a]), &mut cache).unwrap();
        assert_eq!(with("x.yaml#a"), num("1"));
        assert_eq!(with("x.json#a"), unv(Unverifiable::Format));
        assert_eq!(with("h.tf#a"), num("1"));
        assert_eq!(with("h.yaml#a"), MISSING_KEY);
        // Two files with the same content are still two files.
        let mut t = MemTree::new();
        t.file("same/a.yaml", "k: 1\n").file("same/b.yaml", "k: 1\n");
        t.file("hcl/a.tf", "k = 1\n").file("hcl/b.tf", "k = 1\n");
        assert_eq!(res(&t, &["same/*.yaml#k"]), unv(Unverifiable::DefinedTwice));
        assert_eq!(res(&t, &["hcl/*.tf#k"]), unv(Unverifiable::DefinedTwice));
        // A glob of HCL and YAML files: a key in both is defined twice; the first reason in path order wins.
        t.file("mix/a.tf", "k = 1\n").file("mix/b.yaml", "k: 2\n");
        assert_eq!(res(&t, &["mix/*#k"]), unv(Unverifiable::DefinedTwice));
        let pointer = "version https://git-lfs.github.com/spec/v1\noid sha256:4d7a\nsize 1\n";
        t.file("mix/a.tf", pointer).file("mix/b.yaml", b"k: \xff\n");
        assert_eq!(res(&t, &["mix/*#k"]), unv(Unverifiable::LfsPointer));
        t.file("mix/c.tf", "z = 1\n");
        assert_eq!(res(&t, &["mix/*#k"]), unv(Unverifiable::LfsPointer));
        t.file("mix/0.yaml", b"k: \xff\n");
        assert_eq!(res(&t, &["mix/*#k"]), unv(Unverifiable::Encoding));
        // An LFS pointer is judged before the encoding.
        t.file("p.yaml", [pointer.as_bytes(), b"\xff\n"].concat());
        assert_eq!(res(&t, &["p.yaml#k"]), unv(Unverifiable::LfsPointer));
        // The merged HCL value is kept for the pass and answers every key.
        let mut cache = FileCache::default();
        t.file("tf/a.tf", "x = 1\n").file("tf/b.tf", "y = 2\n");
        for (k, want) in [("x", "1"), ("y", "2"), ("x", "1")] {
            assert_eq!(resolve(&t, &layers(&[&format!("tf/*.tf#{k}")]), &mut cache).unwrap(), num(want));
        }
        // A tagged document may hold the key another document holds.
        t.file("d.yaml", "--- !Foo\na: 1\n---\nb: 2\n");
        assert_eq!(res(&t, &["d.yaml#b"]), unv(Unverifiable::Tag));
        assert_eq!(res(&t, &["d.yaml#[1].b"]), num("2"));
        // An index on a map is the map, once.
        t.file("m.yaml", "m: {b: 1}\n");
        assert_eq!(res(&t, &["m.yaml#m[0].b"]), num("1"));
        assert_eq!(res(&t, &["m.yaml#m[1]"]), MISSING_KEY);
        // The first reason in element order, and in criterion order; a criterion that fails decides.
        t.file("s.yaml", "l:\n- {p: on, x: 1}\n- {p: !Ref y, x: 2}\n");
        assert_eq!(res(&t, &["s.yaml#l[p=1].x"]), unv(Unverifiable::Yaml11));
        t.file("s.yaml", "l:\n- {a: on, b: !Ref y, x: 1}\n");
        assert_eq!(res(&t, &["s.yaml#l[a=1,b=1].x"]), unv(Unverifiable::Yaml11));
        t.file("s.yaml", "l:\n- {a: on, b: 2, x: 1}\n- {a: 1, b: 1, x: 2}\n");
        assert_eq!(res(&t, &["s.yaml#l[a=1,b=1].x"]), num("2"));
        // A selector key that goes through a list cannot be compared.
        t.file("s.yaml", "l:\n- {meta: [{name: a}], x: 1}\n");
        assert_eq!(res(&t, &["s.yaml#l[meta.name=a].x"]), unv(Unverifiable::Ambiguous));
        // An executable file is read; no layers is no file.
        t.exec("e.yaml", "a: 1\n");
        assert_eq!(res(&t, &["e.yaml#a"]), num("1"));
        assert_eq!(resolve(&t, &[], &mut FileCache::default()).unwrap(), MISSING_FILE);
        // A later layer whose file lacks the key leaves the earlier value.
        t.file("l/base.yaml", "a: 1\n").file("l/prod.yaml", "b: 2\n");
        assert_eq!(res(&t, &["l/base.yaml#a", "l/prod.yaml#a"]), num("1"));
        assert_eq!(res(&t, &["l/base.yaml#a", "l/none.yaml#a"]), num("1"));
        assert_eq!(res(&t, &["l/none.yaml#a", "l/prod.yaml#a"]), MISSING_KEY);
    }

    proptest::proptest! {
        /// Any bytes in any format never panic, and a warm cache answers as a cold one.
        #[test]
        fn never_panics_and_cache_agrees(
            bytes in proptest::collection::vec(proptest::prop_oneof![
                proptest::strategy::Just(b'a'), proptest::strategy::Just(b':'), proptest::strategy::Just(b' '),
                proptest::strategy::Just(b'\n'), proptest::strategy::Just(b'-'), proptest::strategy::Just(b'['),
                proptest::strategy::Just(b']'), proptest::strategy::Just(b'{'), proptest::strategy::Just(b'}'),
                proptest::strategy::Just(b'='), proptest::strategy::Just(b'"'), proptest::strategy::Just(b'1'),
                proptest::strategy::Just(b','), proptest::strategy::Just(0xff), proptest::num::u8::ANY,
            ], 0..64),
        ) {
            let mut t = MemTree::new();
            for f in ["p/x.yaml", "p/x.json", "p/x.toml", "p/x.tf", "p/y.tf"] {
                t.file(f, &bytes);
            }
            let anchors = ["p/x.yaml#a", "p/x.json#a[0]", "p/x.toml#a.a", "p/*.tf#a", "p/*#a", "p/x.yaml#[a=1].a"];
            let (mut warm, mut none, mut tiny) = (FileCache::default(), FileCache::with_budget(0), FileCache::with_budget(1_024));
            for a in anchors {
                let cold = resolve(&t, &layers(&[a]), &mut FileCache::default()).unwrap();
                proptest::prop_assert_eq!(&resolve(&t, &layers(&[a]), &mut warm).unwrap(), &cold);
                proptest::prop_assert_eq!(&resolve(&t, &layers(&[a]), &mut none).unwrap(), &cold);
                proptest::prop_assert_eq!(&resolve(&t, &layers(&[a]), &mut tiny).unwrap(), &cold);
            }
        }
    }

    #[test]
    fn held_stays_within_the_budget() {
        // C1: a glob over many files keeps at most the budget of them, and answers as an unbounded pass does.
        let mut t = MemTree::new();
        let pad = "x".repeat(1_000);
        for i in 0..8 {
            t.file(&format!("d/{i}.json"), format!("{{\"k{i}\": {i}, \"pad\": \"{pad}\"}}"));
        }
        let ls = layers(&["d/*.json#k7"]);
        let want = resolve(&t, &ls, &mut FileCache::with_budget(u64::MAX)).unwrap();
        assert_eq!(want, num("7"));
        let mut cache = FileCache::with_budget(2_500);
        LET_GO.with(|n| n.set(0));
        for _ in 0..3 {
            assert_eq!(resolve(&t, &ls, &mut cache).unwrap(), want);
            assert!(cache.held() <= 2_500, "held {}", cache.held());
        }
        assert!(LET_GO.with(Cell::get) > 0, "nothing was let go");
        // A file over the budget is never kept, and still answers.
        let mut none = FileCache::with_budget(10);
        assert_eq!(resolve(&t, &ls, &mut none).unwrap(), want);
        assert_eq!(none.held(), 0);
    }

    #[test]
    fn reasons_are_charged_too() {
        // C1, second pass: a reason is kept as well, so it is charged, or a tree of many unreadable files would
        // grow what a pass keeps without bound.
        let mut t = MemTree::new();
        for i in 0..200 {
            t.file(&format!("d/{i:03}.json"), vec![0xff, b'0' + (i % 10) as u8, b'a' + (i / 10) as u8]);
        }
        let mut cache = FileCache::with_budget(4_096);
        assert_eq!(resolve(&t, &layers(&["d/*.json#k"]), &mut cache).unwrap(), unv(Unverifiable::Encoding));
        assert!(cache.held() <= 4_096 && cache.kept.len() < 200, "kept {} for {} bytes", cache.kept.len(), cache.held());
    }

    #[test]
    fn hcl_set_is_bounded_as_one_file() {
        // C1: a glob's HCL files merge into one value, so together they are bounded as one HCL file.
        let mut t = MemTree::new();
        t.file("d/a.tf", "a = 1\n").file("d/b.tf", "b = 2\n");
        let half = anchored_file_max(Format::Hcl) / 2;
        let over = unv(Unverifiable::Bound(Bound::AnchoredFile));
        assert_eq!(res(&Sized(&t, half + 1), &["d/*.tf#a"]), over);
        assert_eq!(res(&Sized(&t, half + 1), &["d/a.tf#a"]), num("1"));
        assert_eq!(res(&Sized(&t, half), &["d/*.tf#a"]), num("1"));
        // Again on the bytes read: each file alone is under the bound, together they are over it.
        let padded = Padded { inner: &t, to: half + 1 };
        assert_eq!(res(&padded, &["d/a.tf#a"]), num("1"));
        assert_eq!(res(&padded, &["d/*.tf#a"]), over);
        // Together exactly at the bound once read (each blob reads `to` + 1 bytes): read.
        assert_eq!(res(&Padded { inner: &t, to: half - 1 }, &["d/*.tf#a"]), num("1"));
    }

    #[test]
    fn the_cache_lets_the_least_recently_used_go() {
        let oid = |s: &str| Oid::sha256_of(s.as_bytes());
        let text = |n: usize| Ok::<Arc<str>, Unverifiable>(Arc::from("x".repeat(n)));
        // Each costs its text, an entry and its key's 64 hex digits: 680 + 256 + 64 = 1,000.
        let mut c = FileCache::with_budget(2_000);
        c.put_code(oid("a"), text(680));
        assert_eq!(c.held(), 1_000);
        c.put_code(oid("b"), text(680));
        assert_eq!(c.held(), 2_000, "two fit exactly");
        assert!(c.code(&oid("a")).is_some(), "a is now the most recently used");
        c.put_code(oid("c"), text(680));
        assert_eq!(c.held(), 2_000);
        assert!(c.code(&oid("b")).is_none(), "b, the least recently used, was let go");
        assert!(c.code(&oid("a")).is_some() && c.code(&oid("c")).is_some());
        // What costs the whole budget is kept; a byte more is not, and lets nothing go.
        let mut one = FileCache::with_budget(1_000);
        one.put_code(oid("d"), text(680));
        assert_eq!(one.held(), 1_000);
        one.put_code(oid("e"), text(681));
        assert!(one.code(&oid("e")).is_none());
        assert!(one.code(&oid("d")).is_some());
    }

    /// Records each prefetch hint; reads as the `MemTree` does.
    struct Hints<'a> {
        inner: &'a MemTree,
        hints: std::cell::RefCell<Vec<Vec<Oid>>>,
    }
    impl Tree for Hints<'_> {
        fn entry(&self, path: &str) -> Result<Option<Entry>, TreeError> {
            self.inner.entry(path)
        }
        fn list(&self, dir: &str) -> Result<Vec<(String, Entry)>, TreeError> {
            self.inner.list(dir)
        }
        fn read_many(&self, oids: &[Oid]) -> Result<Vec<Arc<[u8]>>, TreeError> {
            self.inner.read_many(oids)
        }
        fn prefetch(&self, oids: &[Oid]) {
            self.hints.borrow_mut().push(oids.to_vec());
        }
    }

    #[test]
    fn a_layer_hints_only_the_files_it_does_not_hold() {
        let mut t = MemTree::new();
        t.file("d/a.json", "{\"a\": 1}").file("d/b.tf", "b = 2\n").file("d/c.txt", "c");
        let oid = |p: &str| t.entry(p).unwrap().unwrap().oid;
        let tree = Hints { inner: &t, hints: Default::default() };
        let mut cache = FileCache::default();
        let first = resolve(&tree, &layers(&["d/*#a"]), &mut cache).unwrap();
        // The JSON and the HCL file are read; a file of no format never is.
        assert_eq!(*tree.hints.borrow(), [vec![oid("d/a.json"), oid("d/b.tf")]]);
        // Both held now: the layer again reads them from the cache and hints nothing.
        assert_eq!(resolve(&tree, &layers(&["d/*#a"]), &mut cache).unwrap(), first);
        assert_eq!(tree.hints.borrow().len(), 1);
    }

    /// Every entry reports size 0; each blob reads padded with an HCL comment to `to` bytes.
    struct Padded<'a> {
        inner: &'a MemTree,
        to: u64,
    }
    impl Tree for Padded<'_> {
        fn entry(&self, path: &str) -> Result<Option<Entry>, TreeError> {
            Ok(self.inner.entry(path)?.map(|e| Entry { size: 0, ..e }))
        }
        fn list(&self, dir: &str) -> Result<Vec<(String, Entry)>, TreeError> {
            Ok(self.inner.list(dir)?.into_iter().map(|(p, e)| (p, Entry { size: 0, ..e })).collect())
        }
        fn read_many(&self, oids: &[Oid]) -> Result<Vec<Arc<[u8]>>, TreeError> {
            let blobs = self.inner.read_many(oids)?;
            Ok(blobs
                .into_iter()
                .map(|b| {
                    let mut v = b.to_vec();
                    v.extend_from_slice(b"# ");
                    v.resize(usize::try_from(self.to).unwrap(), b'x');
                    v.push(b'\n');
                    Arc::from(v)
                })
                .collect())
        }
    }

    #[test]
    fn the_budget_never_changes_an_answer() {
        // C1: every status, context and gate case of the corpus answers alike whatever a pass may keep: nothing,
        // or about one small file, so each read lets the one before it go.
        use crate::test_common::{mem_tree, walk};
        let mut files = Vec::new();
        walk(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/conformance/cases")), &mut files);
        let run = |case: &serde_json::Value| -> Option<String> {
            let tree = mem_tree(&case["tree"]).unwrap();
            Some(match case["op"].as_str()? {
                "status" => format!("{:?}", crate::folder::status(&tree)),
                "context" => {
                    let paths: Vec<String> = case["paths"].as_array()?.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect();
                    format!("{:?}", crate::route::context(&tree, &paths, crate::bounds::CONTEXT_CAP))
                }
                "gate" => {
                    let base = mem_tree(&case["base"]).unwrap();
                    let full = crate::gate::gate_full(&base, &tree);
                    format!("{full:?} {:?}", crate::gate::gate(&base, &tree, &base.changed_paths(&tree)))
                }
                _ => return None,
            })
        };
        let (mut ran, mut let_go, mut evicted) = (0, 0, 0);
        for f in files {
            let case: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
            let Some(want) = run(&case) else { continue };
            for budget in [0, 1_024] {
                BUDGET.with(|b| b.set(Some(budget)));
                LET_GO.with(|n| n.set(0));
                let got = run(&case);
                BUDGET.with(|b| b.set(None));
                let n = LET_GO.with(Cell::get);
                let_go += n;
                evicted += if budget > 0 { n } else { 0 };
                assert_eq!(got.as_deref(), Some(want.as_str()), "{} at budget {budget}", f.display());
            }
            ran += 1;
        }
        assert!(ran >= 235, "ran {ran}");
        assert!(let_go > 0 && evicted > 0, "the budget was never exercised: {let_go} let go, {evicted} at 1 KiB");
    }
}
