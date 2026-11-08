// SPDX-License-Identifier: MIT
//! Which trees a command judges: H (a commit, or the working tree through its view), the base B it is compared
//! with, the paths that differ between them, and which record files this branch proposes.

use std::collections::BTreeSet;

use docbrain_knowledge::folder::scope_of;
use docbrain_knowledge::git::{CommitTree, GitError, GitRepo};
use docbrain_knowledge::{ChangedPaths, Oid, Tree, TreeError};

use super::Session;
use super::paths::flat;
use super::worktree::{WorktreeView, worktree_view};

/// The trees one command judges.
pub(crate) struct Judged {
    /// H: the commit named, or the working tree.
    pub h: CommitTree,
    /// H's commit: `<rev>`, else `HEAD`, else `None` on an unborn branch.
    pub h_commit: Option<Oid>,
    /// B, the merge base of H's commit with `--base`, and its id.
    pub base: Option<(CommitTree, Oid)>,
    /// The paths that differ between B and H.
    pub changed: Option<ChangedPaths>,
    /// The tree or commit H is, as `git diff-tree` takes it.
    h_id: Oid,
    /// The working-tree view H reads through, kept while H is read.
    view: Option<WorktreeView>,
}

/// Which record files this branch proposes: added or edited since the reviewed base, or unknown without one.
pub(crate) enum Review {
    Known(BTreeSet<String>),
    Unknown,
}

const BARE: &str = "a bare repository has no working tree: pass a <rev>";
const UNBORN: &str = "no commit yet: --base needs one";

/// H, and with `base` B and the changed paths. The error is the line printed before exit 3.
pub(crate) fn open_trees(s: &Session, rev: Option<&str>, base: Option<&str>) -> Result<Judged, String> {
    let named = rev.map(|rev| s.repo.resolve_commit(rev)).transpose().map_err(git_line)?;
    let h_commit = match &named {
        Some(c) => Some(c.clone()),
        None if s.top.is_none() => return Err(BARE.to_owned()),
        None => match s.repo.resolve_commit("HEAD") {
            Ok(c) => Some(c),
            Err(GitError::NotFound(_)) => None,
            Err(e) => return Err(git_line(e)),
        },
    };
    // B before the working tree: a base that names nothing exits before the whole index is refreshed.
    let base = match base {
        None => None,
        Some(base) => {
            let h_commit = h_commit.as_ref().ok_or_else(|| UNBORN.to_owned())?;
            let b = match s.repo.resolve_commit(base) {
                Ok(b) => b,
                // A shallow clone holds part of the history: a base it lacks is history to fetch, not a typo.
                Err(GitError::NotFound(_)) if s.shallow => return Err(no_merge_base(base)),
                Err(e) => return Err(git_line(e)),
            };
            let m = s.repo.merge_base(h_commit, &b).map_err(|e| match e {
                GitError::NoMergeBase => no_merge_base(base),
                e => git_line(e),
            })?;
            Some((s.repo.commit_tree(&m).map_err(git_line)?, m))
        }
    };
    let mut j = match named {
        Some(c) => Judged { h: s.repo.commit_tree(&c).map_err(git_line)?, h_commit, base, changed: None, h_id: c, view: None },
        None => {
            let view = worktree_view(s).map_err(|e| flat(&format!("{e:#}")))?;
            let h = view.repo.tree(&view.tree).map_err(git_line)?;
            Judged { h, h_commit, base, changed: None, h_id: view.tree.clone(), view: Some(view) }
        }
    };
    if let Some((_, m)) = &j.base {
        j.changed = Some(j.reader(s).changed_paths(m, &j.h_id).map_err(git_line)?);
    }
    Ok(j)
}

impl Judged {
    /// The repository H's objects are read from: the view's overlay for the working tree.
    fn reader<'a>(&'a self, s: &'a Session) -> &'a GitRepo {
        self.view.as_ref().map_or(&s.repo, |v| &v.repo)
    }
}

/// The reviewed base is the merge base of H's commit with `--base`, else with `refs/remotes/origin/HEAD`.
/// Without one, or with several merge bases, the review state is unknown; never an exit.
pub(crate) fn review(s: &Session, j: &Judged, base: Option<&str>) -> Review {
    proposed(s, j, base).map_or(Review::Unknown, Review::Known)
}

fn proposed(s: &Session, j: &Judged, base: Option<&str>) -> Option<BTreeSet<String>> {
    let since_base;
    let changed = match (&j.changed, base) {
        // The gate's B is the merge base with `--base` already.
        (Some(changed), Some(_)) => changed,
        (_, base) => {
            let base = s.repo.resolve_commit(base.unwrap_or("refs/remotes/origin/HEAD")).ok()?;
            let reviewed = s.repo.merge_base(j.h_commit.as_ref()?, &base).ok()?;
            since_base = j.reader(s).changed_paths(&reviewed, &j.h_id).ok()?;
            &since_base
        }
    };
    let mut set = BTreeSet::new();
    for path in changed.paths().iter().filter(|p| p.ends_with(".md") && scope_of(p).is_some()) {
        // A record file deleted since the reviewed base proposes nothing.
        if j.h.entry(path).ok()?.is_some() {
            set.insert(path.clone());
        }
    }
    Some(set)
}

fn no_merge_base(base: &str) -> String {
    format!(
        "no merge base with {base} in the local history: fetch more of it (git fetch --deepen=<n>, or fetch-depth 2 on a merge commit)"
    )
}

fn missing(oid: &Oid) -> String {
    format!("object {oid} is not in the local repository: fetch it, or run where the history is")
}

/// The exit-3 line of a git failure.
fn git_line(e: GitError) -> String {
    match e {
        GitError::NotFound(name) => format!("no commit named {name}"),
        GitError::SeveralBases(bases) => {
            let bases = bases.iter().map(Oid::as_str).collect::<Vec<_>>().join(", ");
            format!("several merge bases ({bases}): pass the base commit with --base")
        }
        GitError::MissingObject(oid) => missing(&oid),
        GitError::NotARepo => "not a git repository: run this inside one".to_owned(),
        GitError::NoMergeBase => "no merge base".to_owned(),
        GitError::Git { command, stderr } => flat(&format!("{command} failed: {stderr}")),
    }
}

/// The exit-3 line of a failure to read H or B.
pub(crate) fn tree_error_line(e: &TreeError) -> String {
    match e {
        TreeError::MissingObject(oid) => missing(oid),
        TreeError::Git { command, stderr } => flat(&format!("{command} failed: {stderr}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::testrepo::Repo;
    use std::path::Path;

    fn open(dir: &Path) -> Session {
        Session::open(dir).unwrap()
    }

    /// The exit-3 line.
    fn err(s: &Session, rev: Option<&str>, base: Option<&str>) -> String {
        open_trees(s, rev, base).err().expect("exits 3")
    }

    fn read(t: &dyn Tree, path: &str) -> Vec<u8> {
        let e = t.entry(path).unwrap().unwrap_or_else(|| panic!("{path}"));
        t.read_many(&[e.oid]).unwrap()[0].to_vec()
    }

    fn changed(j: &Judged) -> Vec<String> {
        j.changed.as_ref().expect("a base was given").paths().to_vec()
    }

    fn known(r: Review) -> Vec<String> {
        match r {
            Review::Known(set) => set.into_iter().collect(),
            Review::Unknown => panic!("review state unknown"),
        }
    }

    fn hint(base: &str) -> String {
        format!(
            "no merge base with {base} in the local history: fetch more of it (git fetch --deepen=<n>, or fetch-depth 2 on a merge commit)"
        )
    }

    /// Branches `a` and `b` merged into each other: two merge bases, `a1` and `b1`.
    fn criss_cross() -> Repo {
        let r = Repo::new();
        r.write("f", "0\n").commit("root");
        r.git(&["checkout", "-q", "-b", "a"]);
        let a1 = r.write("a", "1\n").commit("a1");
        r.git(&["checkout", "-q", "-b", "b", "main"]);
        r.write("b", "1\n").commit("b1");
        r.git(&["checkout", "-q", "a"]);
        r.git(&["merge", "-q", "--no-ff", "-m", "a2", "b"]);
        r.git(&["checkout", "-q", "b"]);
        r.git(&["merge", "-q", "--no-ff", "-m", "b2", &a1]);
        r
    }

    #[test]
    fn rev_form_reads_that_commit() {
        let r = Repo::new();
        let first = r.write("a.yaml", "x: 1\n").commit("first");
        let second = r.write("a.yaml", "x: 2\n").commit("second");
        r.write("a.yaml", "x: 3\n");
        let s = open(r.dir());
        let j = open_trees(&s, Some(&first[..10]), None).unwrap();
        assert_eq!(read(&j.h, "a.yaml"), b"x: 1\n");
        assert_eq!(j.h_commit.as_ref().map(Oid::as_str), Some(first.as_str()));
        assert!(j.view.is_none() && j.base.is_none() && j.changed.is_none());
        let j = open_trees(&s, None, None).unwrap();
        assert_eq!(read(&j.h, "a.yaml"), b"x: 3\n", "without <rev>, the working tree");
        assert_eq!(j.h_commit.as_ref().map(Oid::as_str), Some(second.as_str()), "and HEAD is its commit");
        assert_eq!(err(&s, Some("nope"), None), "no commit named nope");
    }

    #[test]
    fn base_is_the_merge_base_with_rev() {
        let r = Repo::new();
        let root = r.write("a.yaml", "x: 1\n").write("b.yaml", "y: 1\n").commit("root");
        r.git(&["checkout", "-q", "-b", "feat"]);
        r.write("a.yaml", "x: 2\n").commit("feat");
        r.git(&["checkout", "-q", "main"]);
        r.write("b.yaml", "y: 2\n").commit("main moves on");
        let s = open(r.dir());
        let j = open_trees(&s, Some("feat"), Some("main")).unwrap();
        let (b, id) = j.base.as_ref().unwrap();
        assert_eq!(id.as_str(), root);
        assert_eq!(read(b, "b.yaml"), b"y: 1\n");
        assert_eq!(changed(&j), ["a.yaml"]);
        assert_eq!(err(&s, Some("feat"), Some("nope")), "no commit named nope");
    }

    /// B is settled before the working tree is read: a base that names nothing never costs a refresh of the index.
    /// The index is a directory, which stops every reader, root included (a mode of 000 stops no root).
    #[test]
    fn a_bad_base_exits_before_the_working_tree_is_read() {
        let r = Repo::new();
        r.write("a.yaml", "x: 1\n").commit("first");
        let index = r.dir().join(".git/index");
        std::fs::remove_file(&index).unwrap();
        std::fs::create_dir(&index).unwrap();
        let s = open(r.dir());
        let bad = err(&s, None, Some("nope"));
        let unreadable = err(&s, None, Some("HEAD"));
        assert_eq!(bad, "no commit named nope");
        assert!(unreadable.starts_with("cannot ") && unreadable.contains(" the index "), "{unreadable}");
    }

    #[test]
    fn worktree_changed_paths_include_unstaged_and_exclude_untracked() {
        let r = Repo::new();
        r.write("a.yaml", "x: 1\n").write("b.yaml", "y: 1\n").commit("first");
        r.write("a.yaml", "x: 2\n");
        r.write("s.yaml", "staged\n").git(&["add", "s.yaml"]);
        r.write("u.yaml", "untracked\n");
        let j = open_trees(&open(r.dir()), None, Some("HEAD")).unwrap();
        assert_eq!(changed(&j), ["a.yaml", "s.yaml"]);
    }

    #[test]
    fn several_merge_bases_exit_3() {
        let r = criss_cross();
        let bases = r.git(&["merge-base", "--all", "a", "b"]).lines().collect::<Vec<_>>().join(", ");
        assert_eq!(bases.matches(", ").count(), 1, "two merge bases: {bases}");
        let want = format!("several merge bases ({bases}): pass the base commit with --base");
        assert_eq!(err(&open(r.dir()), Some("a"), Some("b")), want);
        r.git(&["checkout", "-q", "a"]);
        assert_eq!(err(&open(r.dir()), None, Some("b")), want, "the working tree's HEAD");
    }

    #[test]
    fn base_missing_from_a_shallow_clone_exits_3() {
        let origin = Repo::new();
        let one = origin.write("a.yaml", "x: 1\n").commit("one");
        origin.write("a.yaml", "x: 2\n").commit("two");
        origin.write("a.yaml", "x: 3\n").commit("three");
        origin.git(&["config", "uploadpack.allowAnySHA1InWant", "true"]);
        let scratch = tempfile::tempdir().unwrap();
        let clone = scratch.path().join("clone");
        let url = format!("file://{}", origin.dir().display());
        origin.git_in(scratch.path(), &["clone", "-q", "--depth", "1", &url, clone.to_str().unwrap()]);
        assert_eq!(err(&open(&clone), None, Some(&one)), hint(&one), "the base is not in the clone");
        origin.git_in(&clone, &["fetch", "-q", "--depth", "1", "origin", &one]);
        assert_eq!(err(&open(&clone), None, Some(&one)), hint(&one), "the base is there, the history joining it is not");
        assert!(open_trees(&open(&clone), None, Some("HEAD")).is_ok());
        // A full clone names a typo as a typo.
        assert_eq!(err(&open(origin.dir()), None, Some("nope")), "no commit named nope");
    }

    #[test]
    fn blobless_clone_offline_exits_3() {
        let origin = Repo::new();
        let old = origin.write(".knowledge/records/r.md", "record\n").write("a.yaml", "x: 1\n").commit("one");
        origin.write("a.yaml", "x: 2\n").commit("two");
        origin.git(&["config", "uploadpack.allowFilter", "true"]);
        origin.git(&["config", "uploadpack.allowAnySHA1InWant", "true"]);
        let scratch = tempfile::tempdir().unwrap();
        let clone = scratch.path().join("clone");
        let url = format!("file://{}", origin.dir().display());
        origin.git_in(scratch.path(), &["clone", "-q", "--filter=blob:none", &url, clone.to_str().unwrap()]);
        let gone = format!("file://{}", scratch.path().join("gone").display());
        origin.git_in(&clone, &["remote", "set-url", "origin", &gone]);
        let j = open_trees(&open(&clone), Some(&old), None).unwrap();
        let e = j.h.entry("a.yaml").unwrap().unwrap();
        let line = j.h.read_many(std::slice::from_ref(&e.oid)).err().map(|e| tree_error_line(&e));
        let want = format!("object {} is not in the local repository: fetch it, or run where the history is", e.oid);
        assert_eq!(line, Some(want));
    }

    #[test]
    fn bare_repository_with_rev_works_and_without_exits_3() {
        let r = Repo::new();
        r.write("a.yaml", "x: 1\n").commit("one");
        let scratch = tempfile::tempdir().unwrap();
        let bare = scratch.path().join("bare.git");
        r.git_in(scratch.path(), &["clone", "-q", "--bare", r.dir().to_str().unwrap(), bare.to_str().unwrap()]);
        let s = open(&bare);
        assert_eq!(read(&open_trees(&s, Some("HEAD"), None).unwrap().h, "a.yaml"), b"x: 1\n");
        // Before any base is read: one that names nothing is never reached.
        for base in [None, Some("HEAD"), Some("no-such-ref")] {
            assert_eq!(err(&s, None, base), "a bare repository has no working tree: pass a <rev>");
        }
    }

    #[test]
    fn unborn_head_with_base_exits_3() {
        let r = Repo::new();
        r.write("a.yaml", "x: 1\n").git(&["add", "a.yaml"]);
        let s = open(r.dir());
        assert_eq!(err(&s, None, Some("main")), "no commit yet: --base needs one");
        let j = open_trees(&s, None, None).unwrap();
        assert!(j.h_commit.is_none());
        assert_eq!(read(&j.h, "a.yaml"), b"x: 1\n");
    }

    #[test]
    fn review_marks_records_added_since_origin_head() {
        let r = Repo::new();
        let reviewed = r
            .write(".knowledge/records/a.md", "a\n")
            .write(".knowledge/records/gone.md", "g\n")
            .write("svc/.knowledge/records/s.md", "s\n")
            .write("x.yaml", "x: 1\n")
            .commit("reviewed");
        r.git(&["update-ref", "refs/remotes/origin/HEAD", &reviewed]);
        r.git(&["checkout", "-q", "-b", "feat"]);
        r.write(".knowledge/records/a.md", "a2\n").write(".knowledge/records/b.md", "b\n").write("x.yaml", "x: 2\n");
        r.git(&["rm", "-q", ".knowledge/records/gone.md"]);
        r.commit("proposed");
        r.write("svc/.knowledge/records/s.md", "s2\n");
        r.write("svc/.knowledge/records/t.md", "t\n").git(&["add", "svc/.knowledge/records/t.md"]);
        r.write("svc/.knowledge/records/u.md", "untracked\n");
        let s = open(r.dir());
        let review_of = |rev: Option<&str>, base: Option<&str>| known(review(&s, &open_trees(&s, rev, base).unwrap(), base));
        let all = [".knowledge/records/a.md", ".knowledge/records/b.md", "svc/.knowledge/records/s.md", "svc/.knowledge/records/t.md"];
        assert_eq!(review_of(None, None), all, "the working tree against origin/HEAD");
        assert_eq!(review_of(Some("feat"), None), all[..2], "a commit against origin/HEAD");
        assert_eq!(review_of(Some("feat"), Some("feat")), Vec::<String>::new(), "--base is the reviewed base");
        assert_eq!(review_of(None, Some("main")), all, "--base main, as origin/HEAD");
    }

    #[test]
    fn a_base_alone_names_the_reviewed_base() {
        let r = Repo::new();
        r.write(".knowledge/records/a.md", "a\n").commit("reviewed");
        r.git(&["checkout", "-q", "-b", "feat"]);
        r.write(".knowledge/records/b.md", "b\n").commit("proposed");
        let s = open(r.dir());
        let alone = |base: &str| review(&s, &open_trees(&s, None, None).unwrap(), Some(base));
        assert_eq!(known(alone("main")), [".knowledge/records/b.md"], "no gate: the base is resolved here");
        assert!(matches!(alone("nope"), Review::Unknown), "a base that names nothing: unknown, never an exit");
        let r = criss_cross();
        r.git(&["checkout", "-q", "a"]);
        let s = open(r.dir());
        let several = review(&s, &open_trees(&s, None, None).unwrap(), Some("b"));
        assert!(matches!(several, Review::Unknown), "several bases");
    }

    #[test]
    fn review_is_unknown_without_origin_head_or_base() {
        let r = Repo::new();
        r.write(".knowledge/records/a.md", "a\n").commit("one");
        r.write(".knowledge/records/b.md", "b\n").commit("two");
        let s = open(r.dir());
        assert!(matches!(review(&s, &open_trees(&s, None, None).unwrap(), None), Review::Unknown));
        assert!(matches!(review(&s, &open_trees(&s, Some("HEAD"), None).unwrap(), None), Review::Unknown));
        let r = Repo::new();
        let one = r.write(".knowledge/records/a.md", "a\n").commit("one");
        r.git(&["update-ref", "refs/remotes/origin/HEAD", &one]);
        r.git(&["checkout", "-q", "--orphan", "fresh"]);
        let s = open(r.dir());
        assert!(matches!(review(&s, &open_trees(&s, None, None).unwrap(), None), Review::Unknown), "an unborn branch");
        // Several merge bases with origin/HEAD: unknown, never an exit.
        let r = criss_cross();
        r.git(&["update-ref", "refs/remotes/origin/HEAD", "b"]);
        r.git(&["checkout", "-q", "a"]);
        let s = open(r.dir());
        assert!(matches!(review(&s, &open_trees(&s, None, None).unwrap(), None), Review::Unknown));
    }
}
