// SPDX-License-Identifier: MIT
//! The working-tree view: the tree `git add -u && git write-tree` would write, computed without writing to the
//! repository. A copy of the index and a temporary object directory take every write, and the repository's own
//! object directory is an alternate, so every later read goes through the overlay and sees both.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};
use docbrain_knowledge::Oid;
use docbrain_knowledge::git::GitRepo;

use super::Session;
use super::paths::rev_parse;

/// The working tree as one tree id, readable through `repo`, which every later read uses.
pub(crate) struct WorktreeView {
    pub repo: GitRepo,
    pub tree: Oid,
    _tmp: tempfile::TempDir,
}

/// The view of the index git is using: `$GIT_INDEX_FILE` inside a hook, else the repository's own.
pub(crate) fn worktree_view(s: &Session) -> anyhow::Result<WorktreeView> {
    view_with_index(s, std::env::var_os("GIT_INDEX_FILE"))
}

/// Settings for the two writing commands: a whole index, never a shared one, no housekeeping, and no refusal of a
/// line-ending conversion git could not undo: the view is read, never checked out.
const QUIET: [&str; 8] =
    ["-c", "core.splitIndex=false", "-c", "gc.auto=0", "-c", "maintenance.auto=false", "-c", "core.safecrlf=false"];

fn view_with_index(s: &Session, index_env: Option<OsString>) -> anyhow::Result<WorktreeView> {
    let Some(top) = &s.top else { bail!("a bare repository has no working tree: pass a <rev>") };
    // A relative `$GIT_INDEX_FILE` is relative to the top of the working tree, where git runs hooks.
    let index = match index_env {
        Some(v) if !v.is_empty() => top.join(v),
        _ => {
            let out = rev_parse(top, &["--git-path", "index"]).map_err(anyhow::Error::msg).context("cannot find the index")?;
            top.join(out.trim_end_matches('\n'))
        }
    };
    let tmp = tempfile::tempdir().context("cannot create a temporary directory for the working-tree view")?;
    let objects = tmp.path().join("objects");
    std::fs::create_dir(&objects).context("cannot create the working-tree view's object directory")?;
    let copy = tmp.path().join("index");
    copy_index(&index, &copy)?;
    let [(k1, v1), (k2, v2)] = s.repo.object_overlay_env(&objects).context("cannot read the repository's object directory")?;
    let envs = [(OsStr::new("GIT_INDEX_FILE"), copy.as_os_str()), (&k1, &v1), (&k2, &v2)];
    // git runs `post-index-change` after any index write: the copy's write is not the repository's, so no hook runs.
    let mut no_hooks = OsString::from("core.hooksPath=");
    no_hooks.push(tmp.path().join("no-hooks"));
    git(top, &no_hooks, &envs, &["add", "-u"])?;
    let printed = git(top, &no_hooks, &envs, &["write-tree"])?;
    let tree = Oid::from_hex(printed.trim_end_matches('\n'))
        .with_context(|| format!("git write-tree printed no tree id: {printed:?}"))?;
    let repo = s.repo.with_object_overlay(&objects).context("cannot read through the working-tree view")?;
    Ok(WorktreeView { repo, tree, _tmp: tmp })
}

/// A copy of the index, keeping its modification time: git reads again every file whose time is not older than the
/// index's own, and a copy dated now would trust a file changed in the same tick as the index was written. No index
/// file is an empty index.
fn copy_index(from: &Path, to: &Path) -> anyhow::Result<()> {
    let modified = match std::fs::metadata(from) {
        Ok(meta) => meta.modified().context("cannot read the index's modification time")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("cannot read the index {}", from.display())),
    };
    std::fs::copy(from, to).with_context(|| format!("cannot copy the index {}", from.display()))?;
    let file = std::fs::File::options().write(true).open(to).context("cannot open the copy of the index")?;
    file.set_modified(modified).context("cannot date the copy of the index")
}

/// One writing git command in `top`, with `QUIET` and the `-c` setting `config`; its stdout, or an error carrying
/// git's stderr.
fn git(top: &Path, config: &OsStr, envs: &[(&OsStr, &OsStr)], args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("git")
        .current_dir(top)
        .args(QUIET)
        .arg("-c")
        .arg(config)
        .args(args)
        .envs(envs.iter().copied())
        .output()
        .context("cannot run git")?;
    if !out.status.success() {
        bail!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    String::from_utf8(out.stdout).with_context(|| format!("git {} printed text that is not UTF-8", args.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::Session;
    use crate::knowledge::testrepo::Repo;
    use docbrain_knowledge::{Mode, Tree};
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// One set-up of a repository and where the command runs in it.
    struct Scenario {
        name: &'static str,
        repo: Repo,
        cwd: PathBuf,
        /// `$GIT_INDEX_FILE` as a hook would set it.
        index_env: Option<OsString>,
        /// A second directory the scenario needs alive (a linked worktree, a submodule's origin).
        _keep: Option<Repo>,
    }

    fn base() -> Repo {
        let r = Repo::new();
        r.write("a.yaml", "x: 1\n").write("d/b.md", "b\n").write("charts/api/values.yaml", "replicaCount: 2\n");
        r.commit("base");
        r
    }

    fn at_top(name: &'static str, repo: Repo) -> Scenario {
        let cwd = repo.dir().to_owned();
        Scenario { name, repo, cwd, index_env: None, _keep: None }
    }

    /// git in `cwd` with the environment this process has, as the view runs it; panics on failure.
    fn user_git(cwd: &Path, env: &[(&str, &OsString)], args: &[&str]) -> String {
        let out = Command::new("git").current_dir(cwd).envs(env.iter().map(|(k, v)| (k, v))).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }

    fn scenarios() -> Vec<Scenario> {
        let mut out = Vec::new();

        let r = base();
        r.write("a.yaml", "x: 2\n");
        out.push(at_top("a modified file", r));

        let r = base();
        std::fs::remove_file(r.dir().join("d/b.md")).unwrap();
        out.push(at_top("a deleted file", r));

        let r = base();
        r.write("n.txt", "new\n").git(&["add", "n.txt"]);
        out.push(at_top("a staged new file", r));

        let r = base();
        r.write("u.txt", "untracked\n");
        out.push(at_top("an untracked file", r));

        let r = base();
        r.write("a.yaml", "x: 2\n").git(&["add", "a.yaml"]);
        r.write("a.yaml", "x: 3\n");
        out.push(at_top("a staged edit, then an unstaged one", r));

        let r = base();
        r.git(&["update-index", "--skip-worktree", "a.yaml"]);
        r.write("a.yaml", "x: 2\n");
        out.push(at_top("skip-worktree", r));

        let r = base();
        r.git(&["update-index", "--assume-unchanged", "a.yaml"]);
        r.write("a.yaml", "x: 2\n");
        out.push(at_top("assume-unchanged", r));

        let r = base();
        r.write("n.txt", "new\n").git(&["add", "-N", "n.txt"]);
        out.push(at_top("intent-to-add", r));

        let r = base();
        r.git(&["checkout", "-q", "-b", "other"]);
        r.write("a.yaml", "x: other\n").commit("other");
        r.git(&["checkout", "-q", "main"]);
        r.write("a.yaml", "x: main\n").commit("main");
        let mut merge = Command::new("git");
        crate::knowledge::testrepo::hermetic(&mut merge, r.home());
        let merged = merge.current_dir(r.dir()).args(["merge", "-q", "other"]).output().unwrap();
        assert!(!merged.status.success(), "the merge conflicts");
        out.push(at_top("an unresolved merge conflict", r));

        let r = Repo::new();
        r.write("u.txt", "untracked\n");
        out.push(at_top("a fresh repository with no index", r));

        // Racily clean: the entry, the working file and the index share one whole second, and stat compares
        // seconds and size alone, so only the index file's own time tells git to read the file again.
        let r = base();
        r.git(&["config", "core.checkStat", "minimal"]);
        r.git(&["config", "core.trustctime", "false"]);
        let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_767_225_600);
        set_mtime(&r.dir().join("a.yaml"), t);
        r.git(&["update-index", "--refresh"]);
        r.write("a.yaml", "x: 2\n");
        set_mtime(&r.dir().join("a.yaml"), t);
        set_mtime(&r.dir().join(".git/index"), t);
        out.push(at_top("a racily clean entry", r));

        let r = base();
        r.git(&["config", "core.splitIndex", "true"]);
        r.git(&["update-index", "--split-index"]);
        r.write("n.txt", "new\n").git(&["add", "n.txt"]);
        r.write("a.yaml", "x: 2\n");
        out.push(at_top("core.splitIndex=true", r));

        let r = base();
        r.git(&["sparse-checkout", "set", "--sparse-index", "d"]);
        r.write("d/b.md", "b2\n");
        out.push(at_top("a sparse index", r));

        // `keep.md` is older than the index, so `add -u` leaves it alone and its blob is in the repository only.
        #[cfg(unix)]
        {
            let r = Repo::at("a:b;c\"d\\e");
            r.write("a.yaml", "x: 1\n").write("keep.md", "kept\n").commit("first");
            set_mtime(&r.dir().join("keep.md"), std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_767_225_600));
            r.git(&["update-index", "--refresh"]);
            r.write("a.yaml", "x: 2\n");
            out.push(at_top("a repository whose path holds alternate separators", r));
        }

        // A repository that borrows its objects: the alternates of the repository's own object directory still read.
        let origin = base();
        let r = Repo::new();
        std::fs::remove_dir_all(r.dir().join(".git")).unwrap();
        r.git_in(origin.dir(), &["clone", "-q", "--shared", ".", r.dir().to_str().unwrap()]);
        set_mtime(&r.dir().join("d/b.md"), std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_767_225_600));
        r.git(&["update-index", "--refresh"]);
        r.write("a.yaml", "x: 2\n");
        out.push(Scenario { name: "a repository whose objects are borrowed", cwd: r.dir().to_owned(), repo: r, index_env: None, _keep: Some(origin) });

        // git runs `post-index-change` after any index write; the view's write is not the repository's.
        #[cfg(unix)]
        {
            let r = base();
            let hook = r.dir().join(".git/hooks/post-index-change");
            r.write(".git/hooks/post-index-change", "#!/bin/sh\necho ran >> \"$(git rev-parse --git-dir)/hook-ran\"\n");
            let mut perms = std::fs::metadata(&hook).unwrap().permissions();
            std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
            std::fs::set_permissions(&hook, perms).unwrap();
            r.write("a.yaml", "x: 2\n");
            out.push(at_top("a post-index-change hook", r));
        }

        let main = base();
        let linked = Repo::new();
        let wt = linked.dir().join("wt");
        main.git(&["worktree", "add", "-q", "-b", "side", wt.to_str().unwrap()]);
        std::fs::write(wt.join("a.yaml"), "x: side\n").unwrap();
        out.push(Scenario { name: "a linked worktree", repo: main, cwd: wt, index_env: None, _keep: Some(linked) });

        let r = Repo::new();
        r.git(&["config", "core.autocrlf", "true"]);
        r.write("a.yaml", "x: 1\n").commit("lf");
        r.write("a.yaml", "x: 2\r\ny: 3\r\n");
        out.push(at_top("core.autocrlf=true with a CRLF working file", r));

        let r = base();
        std::fs::copy(r.dir().join(".git/index"), r.dir().join(".git/second-index")).unwrap();
        r.write("n.txt", "new\n");
        let mut add = Command::new("git");
        crate::knowledge::testrepo::hermetic(&mut add, r.home());
        assert!(add.current_dir(r.dir()).env("GIT_INDEX_FILE", ".git/second-index").args(["add", "n.txt"]).status().unwrap().success());
        r.write("a.yaml", "x: 2\n");
        let mut sc = at_top("$GIT_INDEX_FILE names a second index", r);
        sc.index_env = Some(".git/second-index".into());
        out.push(sc);

        let inner = Repo::new();
        inner.write("lib.rs", "fn a() {}\n").commit("inner");
        let r = base();
        r.git(&["-c", "protocol.file.allow=always", "submodule", "add", "-q", inner.dir().to_str().unwrap(), "sub"]);
        r.commit("with a submodule");
        let sub = r.dir().join("sub");
        std::fs::write(sub.join("lib.rs"), "fn b() {}\n").unwrap();
        out.push(Scenario { name: "a submodule as the current directory", repo: r, cwd: sub, index_env: None, _keep: Some(inner) });

        out
    }

    fn set_mtime(p: &Path, t: std::time::SystemTime) {
        std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
    }

    /// The path and bytes of every file under `dir`.
    fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_owned()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.insert(p.clone(), std::fs::read(&p).unwrap());
                }
            }
        }
        out
    }

    /// The tree real git writes from the same place, with the same index.
    fn git_writes(sc: &Scenario) -> String {
        let env: Vec<(&str, &OsString)> = sc.index_env.iter().map(|v| ("GIT_INDEX_FILE", v)).collect();
        user_git(&sc.cwd, &env, &["add", "-u"]);
        user_git(&sc.cwd, &env, &["write-tree"]).trim().to_owned()
    }

    /// The view's tree id is the one git writes, and every blob in it reads through the view, before git has
    /// written anything to the repository that could stand in for the overlay.
    #[test]
    fn matches_git_itself() {
        for sc in scenarios() {
            let s = Session::open(&sc.cwd).unwrap();
            let view = view_with_index(&s, sc.index_env.clone()).unwrap_or_else(|e| panic!("{}: {e:#}", sc.name));
            let tree = view.repo.tree(&view.tree).unwrap_or_else(|e| panic!("{}: {e}", sc.name));
            let files: Vec<_> = tree.list("").unwrap().into_iter().filter(|(_, e)| !matches!(e.mode, Mode::Submodule)).collect();
            let oids: Vec<_> = files.iter().map(|(_, e)| e.oid.clone()).collect();
            let blobs = tree.read_many(&oids).unwrap_or_else(|e| panic!("{}: {e}", sc.name));
            let want = git_writes(&sc);
            assert_eq!(view.tree.as_str(), want, "{}", sc.name);
            let env: Vec<(&str, &OsString)> = sc.index_env.iter().map(|v| ("GIT_INDEX_FILE", v)).collect();
            for ((path, e), bytes) in files.iter().zip(&blobs) {
                assert_eq!(**bytes, *user_git(&sc.cwd, &env, &["cat-file", "blob", e.oid.as_str()]).as_bytes(), "{}: {path}", sc.name);
            }
            eprintln!("scenario '{}': view tree {} equals git's; {} blobs read through the view", sc.name, view.tree, files.len());
        }
    }

    #[test]
    fn the_repository_is_unchanged() {
        for sc in scenarios() {
            let git_dir = sc.repo.dir().join(".git");
            let before = snapshot(&git_dir);
            let s = Session::open(&sc.cwd).unwrap();
            let view = view_with_index(&s, sc.index_env.clone()).unwrap_or_else(|e| panic!("{}: {e:#}", sc.name));
            drop(view);
            assert!(snapshot(&git_dir) == before, "{}: a file under .git changed", sc.name);
        }
    }

    #[test]
    fn reads_go_through_the_view() {
        // An anchor outside a sparse cone is read from the index's objects (edge case 78); a staged-only blob is in
        // the temporary object directory alone.
        let r = base();
        r.git(&["sparse-checkout", "set", "--sparse-index", "d"]);
        r.write("d/new.md", "only in the working tree\n").git(&["add", "-N", "d/new.md"]);
        let s = Session::open(r.dir()).unwrap();
        let view = worktree_view(&s).unwrap();
        let tree = view.repo.tree(&view.tree).unwrap();
        for (path, bytes) in [("charts/api/values.yaml", &b"replicaCount: 2\n"[..]), ("d/new.md", b"only in the working tree\n")] {
            let e = tree.entry(path).unwrap().unwrap_or_else(|| panic!("{path}"));
            assert_eq!(&*tree.read_many(&[e.oid]).unwrap()[0], bytes, "{path}");
        }
        assert!(tree.entry("u.txt").unwrap().is_none());
    }

    #[test]
    fn no_lfs_needed() {
        let r = Repo::new();
        r.write(".gitattributes", "*.bin filter=lfs diff=lfs merge=lfs -text\n").write("a.bin", "raw\n").write("a.yaml", "x: 1\n");
        r.commit("lfs attributes, no lfs");
        r.write("a.yaml", "x: 2\n");
        let sc = at_top("lfs attributes", r);
        let s = Session::open(&sc.cwd).unwrap();
        let view = worktree_view(&s).unwrap();
        assert_eq!(view.tree.as_str(), git_writes(&sc));
    }

    /// An exported empty `$GIT_INDEX_FILE` is no index named: git would read an empty one and leave `.lock` where it
    /// runs; the view reads the repository's own.
    #[test]
    fn an_empty_index_variable_is_the_repositorys_own_index() {
        let r = base();
        r.write("a.yaml", "x: 2\n");
        let s = Session::open(r.dir()).unwrap();
        let own = view_with_index(&s, None).unwrap();
        let empty = view_with_index(&s, Some(OsString::new())).unwrap();
        assert_eq!(empty.tree, own.tree);
    }

    #[test]
    fn a_bare_repository_has_no_view() {
        let r = Repo::new();
        let bare = tempfile::tempdir().unwrap();
        r.git(&["init", "-q", "--bare", bare.path().to_str().unwrap()]);
        let s = Session::open(bare.path()).unwrap();
        assert_eq!(worktree_view(&s).err().map(|e| e.to_string()).as_deref(), Some("a bare repository has no working tree: pass a <rev>"));
    }
}
