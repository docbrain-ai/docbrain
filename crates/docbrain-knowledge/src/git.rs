// SPDX-License-Identifier: MIT
//! Commit trees and changed paths through the repository's own git (`SEMANTICS.md` §The gate, §Cost).
//!
//! Every object is read through one long-lived `git cat-file --batch` per tree with lazy fetch off. The objects
//! of one read, or of one prefetch, that a partial clone does not hold are fetched in one call, as git's own lazy
//! fetch does, and an object still absent when read is `MissingObject`. Trees are parsed here and walked by name, so no pathspec ever reaches
//! git. Git keeps no size in a tree: every blob entry reports size 0 and every bound is judged on the bytes read;
//! a blob is kept to `ANCHORED_READ_MAX + 1` bytes.

use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::sync::Arc;

use crate::bounds::ANCHORED_READ_MAX;
use crate::tree::{ChangedPaths, Entry, Mode, Oid, Tree, TreeError};

/// The most of one blob a read keeps: past every bound.
const READ_CAP: u64 = ANCHORED_READ_MAX + 1;
const CAT: &str = "git cat-file --batch";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitError {
    #[error("not a git repository")]
    NotARepo,
    #[error("no commit named {0:?}")]
    NotFound(String),
    #[error("several merge bases: {}", join(.0))]
    SeveralBases(Vec<Oid>),
    #[error("no merge base")]
    NoMergeBase,
    #[error("object {0} is not in the repository")]
    MissingObject(Oid),
    #[error("`{command}` failed: {stderr}")]
    Git { command: String, stderr: String },
}

fn join(oids: &[Oid]) -> String {
    oids.iter().map(Oid::as_str).collect::<Vec<_>>().join(", ")
}

impl From<GitError> for TreeError {
    fn from(e: GitError) -> TreeError {
        match e {
            GitError::MissingObject(o) => TreeError::MissingObject(o),
            GitError::Git { command, stderr } => TreeError::Git { command, stderr },
            other => TreeError::Git { command: "git".to_owned(), stderr: other.to_string() },
        }
    }
}

fn failed(args: &[&str], stderr: &[u8]) -> GitError {
    GitError::Git { command: format!("git {}", args.join(" ")), stderr: String::from_utf8_lossy(stderr).trim().to_owned() }
}

/// A repository, found from a directory as git finds it.
#[derive(Debug, Clone)]
pub struct GitRepo {
    git_dir: PathBuf,
    cwd: PathBuf,
}

impl GitRepo {
    /// The repository at or above `start`, with the caller's environment (`GIT_DIR` in a hook included).
    pub fn discover(start: &Path) -> Result<GitRepo, GitError> {
        let args = ["rev-parse", "--absolute-git-dir"];
        let out = Command::new("git").current_dir(start).env("LC_ALL", "C").args(args).output().map_err(|e| failed(&args, e.to_string().as_bytes()))?;
        if !out.status.success() {
            let e = failed(&args, &out.stderr);
            return Err(match &e {
                GitError::Git { stderr, .. } if stderr.contains("not a git repository") => GitError::NotARepo,
                _ => e,
            });
        }
        let dir = String::from_utf8(out.stdout).map_err(|_| failed(&args, b"the git directory is not UTF-8"))?;
        Ok(GitRepo { git_dir: PathBuf::from(dir.trim_end_matches('\n')), cwd: start.to_owned() })
    }

    /// `git` pinned to this repository: messages in English, no pager, no replace objects.
    fn git(&self) -> Command {
        let mut c = Command::new("git");
        c.current_dir(&self.cwd).env("GIT_DIR", &self.git_dir).env("LC_ALL", "C").args(["--no-pager", "--no-replace-objects"]).stdin(Stdio::null());
        c
    }

    fn run(&self, args: &[&str]) -> Result<Output, GitError> {
        self.git().args(args).output().map_err(|e| failed(args, e.to_string().as_bytes()))
    }

    /// `rev-parse` of `spec`; `NotFound(name)` when it names nothing of that type.
    fn peel(&self, spec: &str, name: &str) -> Result<Oid, GitError> {
        let args = ["rev-parse", "--verify", "--quiet", "--end-of-options", spec];
        let out = self.run(&args)?;
        match out.status.code() {
            Some(0) => Oid::from_hex(String::from_utf8_lossy(&out.stdout).trim()).ok_or_else(|| failed(&args, b"no object name")),
            Some(1) => Err(GitError::NotFound(name.to_owned())),
            _ => Err(failed(&args, &out.stderr)),
        }
    }

    /// The commit `rev` names; never read as an option.
    pub fn resolve_commit(&self, rev: &str) -> Result<Oid, GitError> {
        if rev.is_empty() {
            return Err(GitError::NotFound(String::new()));
        }
        self.peel(&format!("{rev}^{{commit}}"), rev)
    }

    /// The one merge base of `a` and `b`.
    pub fn merge_base(&self, a: &Oid, b: &Oid) -> Result<Oid, GitError> {
        let args = ["merge-base", "--all", a.as_str(), b.as_str()];
        let out = self.run(&args)?;
        let mut bases: Vec<Oid> = String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| Oid::from_hex(l.trim())).collect();
        match (out.status.code(), bases.len()) {
            (Some(0), 1) => Ok(bases.remove(0)),
            (Some(0), 2..) => Err(GitError::SeveralBases(bases)),
            (Some(1), 0) => Err(GitError::NoMergeBase),
            _ => Err(failed(&args, &out.stderr)),
        }
    }

    /// The tree of `commit`, read lazily.
    pub fn commit_tree(&self, commit: &Oid) -> Result<CommitTree, GitError> {
        let root = self.peel(&format!("{commit}^{{commit}}^{{tree}}"), commit.as_str())?;
        Ok(CommitTree {
            repo: self.clone(),
            raw_len: root.as_str().len() / 2,
            root,
            cat: RefCell::new(None),
            check: RefCell::new(None),
            trees: RefCell::new(HashMap::new()),
            promisors: OnceCell::new(),
        })
    }

    /// Every path added, deleted, modified or changed in mode between the trees of `b` and `h`; a move is its
    /// two paths (`--no-renames`).
    pub fn changed_paths(&self, b: &Oid, h: &Oid) -> Result<ChangedPaths, GitError> {
        let args = ["diff-tree", "-r", "-z", "--no-renames", "--name-only", b.as_str(), h.as_str()];
        let out = self.run(&args)?;
        if !out.status.success() {
            return Err(failed(&args, &out.stderr));
        }
        let paths = out.stdout.split(|&c| c == 0).filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).collect();
        Ok(ChangedPaths::new(paths))
    }

    /// The promisor remotes in the order git tries them: `remote.<name>.promisor` in config order, the remote
    /// named by `extensions.partialClone` last.
    fn promisor_remotes(&self) -> Vec<String> {
        let Ok(out) = self.run(&["config", "-z", "--get-regexp", r"^(remote\..+\.promisor|extensions\.partialclone)$"]) else {
            return Vec::new();
        };
        let (mut names, mut last) = (Vec::<String>::new(), None);
        for item in out.stdout.split(|&c| c == 0).filter(|i| !i.is_empty()) {
            let item = String::from_utf8_lossy(item);
            let (key, value) = item.split_once('\n').unwrap_or((&item, "true"));
            if key == "extensions.partialclone" {
                last = Some(value.to_owned());
            } else if let Some(name) = key.strip_prefix("remote.").and_then(|k| k.strip_suffix(".promisor"))
                && matches!(value.to_ascii_lowercase().as_str(), "true" | "yes" | "on" | "1")
                && !names.iter().any(|n| n == name)
            {
                names.push(name.to_owned());
            }
        }
        if let Some(l) = last {
            names.retain(|n| *n != l);
            names.push(l);
        }
        names
    }

    /// Fetches `oids` from `remote` in one call, as git's own lazy fetch does. A failure is not reported here:
    /// the objects are looked for again.
    fn fetch(&self, remote: &str, oids: &[Oid]) {
        let mut c = self.git();
        c.args(["-c", "fetch.negotiationAlgorithm=noop", "fetch", "--no-tags", "--no-write-fetch-head", "--recurse-submodules=no"])
            .args(["--filter=blob:none", "--stdin", "--end-of-options", remote])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let Ok(mut child) = c.spawn() else { return };
        if let Some(mut stdin) = child.stdin.take() {
            let list: String = oids.iter().map(|o| format!("{o}\n")).collect();
            // The child writes nothing we read, so writing it all first cannot block on its output.
            let _ = stdin.write_all(list.as_bytes());
        }
        let _ = child.wait();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Node {
    Tree,
    Leaf(Mode),
}

type Listing = Arc<BTreeMap<String, (Node, Oid)>>;

/// The tree of one commit. Not shared across threads: it owns one `cat-file` child.
pub struct CommitTree {
    repo: GitRepo,
    root: Oid,
    /// Bytes of an object name in a raw tree: 20 for SHA-1, 32 for SHA-256.
    raw_len: usize,
    cat: RefCell<Option<Cat>>,
    /// A `--batch-check` child, started only in a partial clone, for what a prefetch must fetch.
    check: RefCell<Option<Cat>>,
    trees: RefCell<HashMap<Oid, Listing>>,
    promisors: OnceCell<Vec<String>>,
}

impl CommitTree {
    /// The objects of `oids`, of type `kind`, each kept to `cap` bytes. Those absent are fetched, in one call per
    /// promisor remote, and one still absent is `MissingObject`.
    fn objects(&self, oids: &[Oid], kind: &str, cap: u64) -> Result<HashMap<Oid, Arc<[u8]>>, TreeError> {
        let mut want = oids.to_vec();
        want.sort();
        want.dedup();
        let mut got = HashMap::with_capacity(want.len());
        self.read_present(&mut want, &mut got, kind, cap)?;
        if !want.is_empty() {
            for remote in self.promisors.get_or_init(|| self.repo.promisor_remotes()) {
                self.repo.fetch(remote, &want);
                self.read_present(&mut want, &mut got, kind, cap)?;
                if want.is_empty() {
                    break;
                }
            }
        }
        match want.first() {
            Some(o) => Err(TreeError::MissingObject(o.clone())),
            None => Ok(got),
        }
    }

    /// Reads every object of `want` the repository holds into `got`, leaving the absent ones in `want`. A broken
    /// child is dropped, and the next read starts another.
    fn read_present(&self, want: &mut Vec<Oid>, got: &mut HashMap<Oid, Arc<[u8]>>, kind: &str, cap: u64) -> Result<(), TreeError> {
        let mut slot = self.cat.borrow_mut();
        if slot.is_none() {
            *slot = Some(Cat::spawn(&self.repo, "--batch")?);
        }
        let Some(cat) = slot.as_mut() else { return Err(cat_error("no child")) };
        let mut absent = Vec::new();
        let mut read = || {
            for o in want.drain(..) {
                match cat.get(&o, cap).map_err(|e| cat_error(&e.to_string()))? {
                    Some((ty, bytes)) if ty == kind => {
                        got.insert(o, bytes.into());
                    }
                    Some((ty, _)) => return Err(cat_error(&format!("{o} is a {ty}, not a {kind}"))),
                    None => absent.push(o),
                }
            }
            Ok(())
        };
        let r = read();
        if r.is_err() {
            *slot = None;
        }
        *want = absent;
        r
    }

    /// Those of `oids` the repository does not hold, asked without reading any. A broken child is dropped.
    fn absent(&self, oids: &[Oid]) -> io::Result<Vec<Oid>> {
        let mut slot = self.check.borrow_mut();
        if slot.is_none() {
            *slot = Some(Cat::spawn(&self.repo, "--batch-check").map_err(|e| io::Error::other(e.to_string()))?);
        }
        let Some(check) = slot.as_mut() else { return Err(io::Error::other("no child")) };
        let mut absent = Vec::new();
        for o in oids {
            match check.held(o) {
                Ok(true) => {}
                Ok(false) => absent.push(o.clone()),
                Err(e) => {
                    *slot = None;
                    return Err(e);
                }
            }
        }
        Ok(absent)
    }

    /// One tree object's entries by name, cached.
    fn tree(&self, oid: &Oid) -> Result<Listing, TreeError> {
        if let Some(t) = self.trees.borrow().get(oid) {
            return Ok(t.clone());
        }
        let got = self.objects(std::slice::from_ref(oid), "tree", u64::MAX)?;
        let bytes = got.get(oid).ok_or_else(|| TreeError::MissingObject(oid.clone()))?;
        let t = Arc::new(parse_tree(bytes, self.raw_len).ok_or_else(|| cat_error(&format!("tree {oid} does not parse")))?);
        self.trees.borrow_mut().insert(oid.clone(), t.clone());
        Ok(t)
    }

    /// The node at exactly `path`, walking trees by name; nothing is followed through a link or a submodule.
    fn find(&self, path: &str) -> Result<Option<(Node, Oid)>, TreeError> {
        if path.is_empty() {
            return Ok(None);
        }
        let mut at = self.root.clone();
        let mut parts = path.split('/').peekable();
        while let Some(name) = parts.next() {
            let t = self.tree(&at)?;
            let Some((node, oid)) = t.get(name) else { return Ok(None) };
            if parts.peek().is_none() {
                return Ok(Some((*node, oid.clone())));
            }
            if *node != Node::Tree {
                return Ok(None);
            }
            at = oid.clone();
        }
        Ok(None)
    }
}

fn cat_error(stderr: &str) -> TreeError {
    TreeError::Git { command: CAT.to_owned(), stderr: stderr.to_owned() }
}

/// A raw tree object: `<octal mode> <name>\0<raw object name>` repeated. Modes as git reads them; a name that
/// is not UTF-8 is kept lossily.
fn parse_tree(mut b: &[u8], raw_len: usize) -> Option<BTreeMap<String, (Node, Oid)>> {
    let mut out = BTreeMap::new();
    while !b.is_empty() {
        let sp = b.iter().position(|&c| c == b' ')?;
        let mode = u32::from_str_radix(std::str::from_utf8(&b[..sp]).ok()?, 8).ok()?;
        b = &b[sp + 1..];
        let nul = b.iter().position(|&c| c == 0)?;
        let name = String::from_utf8_lossy(&b[..nul]).into_owned();
        b = &b[nul + 1..];
        let raw = b.get(..raw_len)?;
        let oid = Oid::from_hex(&hex::encode(raw))?;
        b = &b[raw_len..];
        let node = match mode & 0o170_000 {
            0o040_000 => Node::Tree,
            0o100_000 if mode & 0o100 != 0 => Node::Leaf(Mode::Executable),
            0o100_000 => Node::Leaf(Mode::File),
            0o120_000 => Node::Leaf(Mode::Symlink),
            0o160_000 => Node::Leaf(Mode::Submodule),
            _ => return None,
        };
        out.insert(name, (node, oid));
    }
    Some(out)
}

impl Tree for CommitTree {
    fn entry(&self, path: &str) -> Result<Option<Entry>, TreeError> {
        Ok(match self.find(path)? {
            Some((Node::Leaf(mode), oid)) => Some(Entry { mode, oid, size: 0 }),
            _ => None,
        })
    }

    fn list(&self, dir: &str) -> Result<Vec<(String, Entry)>, TreeError> {
        let start = if dir.is_empty() {
            Some((self.root.clone(), String::new()))
        } else {
            match self.find(dir)? {
                Some((Node::Tree, oid)) => Some((oid, format!("{dir}/"))),
                _ => None,
            }
        };
        let (mut out, mut stack) = (Vec::new(), Vec::from_iter(start));
        while let Some((oid, prefix)) = stack.pop() {
            for (name, (node, o)) in self.tree(&oid)?.iter() {
                let p = format!("{prefix}{name}");
                match node {
                    Node::Tree => stack.push((o.clone(), format!("{p}/"))),
                    Node::Leaf(mode) => out.push((p, Entry { mode: *mode, oid: o.clone(), size: 0 })),
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    fn read_many(&self, oids: &[Oid]) -> Result<Vec<Arc<[u8]>>, TreeError> {
        let got = self.objects(oids, "blob", READ_CAP)?;
        oids.iter().map(|o| got.get(o).cloned().ok_or_else(|| TreeError::MissingObject(o.clone()))).collect()
    }

    /// In a partial clone, fetches the blobs of `oids` it does not hold in one call per promisor remote, reading
    /// none of them; anywhere else, nothing.
    fn prefetch(&self, oids: &[Oid]) {
        let remotes = self.promisors.get_or_init(|| self.repo.promisor_remotes());
        if remotes.is_empty() {
            return;
        }
        let mut want = oids.to_vec();
        want.sort();
        want.dedup();
        let Ok(mut absent) = self.absent(&want) else { return };
        for remote in remotes {
            if absent.is_empty() {
                return;
            }
            self.repo.fetch(remote, &absent);
            let Ok(still) = self.absent(&absent) else { return };
            absent = still;
        }
    }
}

/// A `git cat-file --batch` (or `--batch-check`) child that never fetches: one request, one answer.
struct Cat {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Cat {
    fn spawn(repo: &GitRepo, mode: &str) -> Result<Cat, TreeError> {
        let mut c = repo.git();
        c.args(["cat-file", mode]).env("GIT_NO_LAZY_FETCH", "1").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = c.spawn().map_err(|e| cat_error(&e.to_string()))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(cat_error("no pipes"));
        };
        Ok(Cat { child, stdin, stdout: BufReader::new(stdout) })
    }

    /// Whether the repository holds the object, from a `--batch-check` child: one header line per request.
    fn held(&mut self, oid: &Oid) -> io::Result<bool> {
        self.stdin.write_all(format!("{oid}\n").as_bytes())?;
        let mut header = String::new();
        if self.stdout.read_line(&mut header)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "git cat-file exited"));
        }
        Ok(header.trim_end() != format!("{oid} missing"))
    }

    /// One object: its type and its first `cap` bytes (the rest is read and dropped), or `None` when absent.
    fn get(&mut self, oid: &Oid, cap: u64) -> io::Result<Option<(String, Vec<u8>)>> {
        let bad = |h: &str| io::Error::new(io::ErrorKind::InvalidData, format!("unexpected answer {h:?}"));
        self.stdin.write_all(format!("{oid}\n").as_bytes())?;
        let mut header = String::new();
        if self.stdout.read_line(&mut header)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "git cat-file exited"));
        }
        let mut f = header.trim_end_matches('\n').split(' ');
        let (Some(_), Some(ty), size, None) = (f.next(), f.next(), f.next(), f.next()) else { return Err(bad(&header)) };
        let Some(size) = size else { return if ty == "missing" { Ok(None) } else { Err(bad(&header)) } };
        let size: u64 = size.parse().map_err(|_| bad(&header))?;
        let keep = size.min(cap);
        let mut buf = vec![0; usize::try_from(keep).map_err(|_| bad(&header))?];
        self.stdout.read_exact(&mut buf)?;
        let rest = size - keep;
        if io::copy(&mut (&mut self.stdout).take(rest), &mut io::sink())? != rest {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "object cut short"));
        }
        let mut nl = [0u8; 1];
        self.stdout.read_exact(&mut nl)?;
        Ok(Some((ty.to_owned(), buf)))
    }
}

impl Drop for Cat {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_tree_modes_and_garbage() {
        let raw = |mode: &str, name: &str| [format!("{mode} {name}\0").into_bytes(), vec![0xab; 20]].concat();
        let b = [raw("100644", "f"), raw("100755", "x"), raw("120000", "l"), raw("160000", "s"), raw("40000", "d"), raw("100664", "old")].concat();
        let t = parse_tree(&b, 20).unwrap();
        let mode = |n: &str| t.get(n).map(|(k, _)| *k);
        assert_eq!(mode("f"), Some(Node::Leaf(Mode::File)));
        assert_eq!(mode("x"), Some(Node::Leaf(Mode::Executable)));
        assert_eq!(mode("l"), Some(Node::Leaf(Mode::Symlink)));
        assert_eq!(mode("s"), Some(Node::Leaf(Mode::Submodule)));
        assert_eq!(mode("d"), Some(Node::Tree));
        assert_eq!(mode("old"), Some(Node::Leaf(Mode::File)));
        assert_eq!(t["f"].1.as_str(), "ab".repeat(20));
        // Cut short, an unknown mode, a mode that is not octal: no tree.
        for bad in [&b[..b.len() - 1], &raw("170000", "z")[..], &raw("10x644", "z")[..], b"100644 f"] {
            assert_eq!(parse_tree(bad, 20), None);
        }
        assert_eq!(parse_tree(b"", 20), Some(BTreeMap::new()));
        // A name that is not UTF-8 is kept lossily.
        let odd = [b"100644 a\xffb\0".to_vec(), vec![1; 20]].concat();
        assert!(parse_tree(&odd, 20).unwrap().contains_key("a\u{fffd}b"));
    }

    fn init() -> (tempfile::TempDir, GitRepo) {
        let d = tempfile::tempdir().unwrap();
        assert!(Command::new("git").current_dir(d.path()).args(["init", "-q"]).status().unwrap().success());
        let repo = GitRepo::discover(d.path()).unwrap();
        (d, repo)
    }

    #[test]
    fn promisor_remotes_in_order_once() {
        let (d, repo) = init();
        let set = |args: &[&str]| assert!(Command::new("git").current_dir(d.path()).arg("config").args(args).status().unwrap().success());
        // b twice, a, c off, and b named by extensions.partialClone: tried last.
        set(&["remote.b.promisor", "true"]);
        set(&["remote.a.promisor", "yes"]);
        set(&["--add", "remote.b.promisor", "on"]);
        set(&["remote.c.promisor", "false"]);
        set(&["extensions.partialClone", "b"]);
        assert_eq!(repo.promisor_remotes(), ["a", "b"]);
    }

    #[test]
    fn held_says_what_the_repository_holds() {
        let (d, repo) = init();
        let mut w = Command::new("git").current_dir(d.path()).args(["hash-object", "-w", "--stdin"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        w.stdin.take().unwrap().write_all(b"x").unwrap();
        let have = Oid::from_hex(String::from_utf8(w.wait_with_output().unwrap().stdout).unwrap().trim()).unwrap();
        let missing = Oid::from_hex(&"ab".repeat(20)).unwrap();
        let mut check = Cat::spawn(&repo, "--batch-check").unwrap();
        assert!(check.held(&have).unwrap());
        assert!(!check.held(&missing).unwrap());
        assert!(check.held(&have).unwrap(), "one answer per request");
    }

    #[test]
    fn a_dropped_child_is_reaped() {
        let (_d, repo) = init();
        for mode in ["--batch", "--batch-check"] {
            let cat = Cat::spawn(&repo, mode).unwrap();
            let pid = cat.child.id().to_string();
            drop(cat);
            // A child still running, or exited and not waited for, is still in the process table.
            let out = Command::new("ps").args(["-o", "stat=", "-p", &pid]).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "", "{mode}");
        }
    }
}
