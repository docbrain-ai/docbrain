// SPDX-License-Identifier: MIT
//! The repository a command runs in, and its path arguments: relative to the current directory as git takes
//! them, an absolute path accepted inside the work tree, every one turned into the root-relative form the crate's
//! paths have (`SEMANTICS.md` §Paths and globs).

use std::path::{Component, Path, PathBuf};
use std::process::Command;

use docbrain_knowledge::git::{GitError, GitRepo};

/// The repository found from the current directory.
pub(crate) struct Session {
    pub repo: GitRepo,
    /// The work tree's top, as git resolves it; `None` in a bare repository.
    pub top: Option<PathBuf>,
    /// The current directory relative to the top, with a trailing `/` (`""` at the top), as `git rev-parse
    /// --show-prefix` prints it.
    pub prefix: String,
    /// A shallow clone: history past its boundary is not here.
    pub shallow: bool,
}

/// Why a path argument names nothing in the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathRefusal {
    Outside,
    UnderGit,
}

impl PathRefusal {
    /// The one line that says so, naming the argument as typed.
    pub(crate) fn line(self, arg: &str) -> String {
        match self {
            PathRefusal::Outside => format!("{arg} is outside the repository"),
            PathRefusal::UnderGit => format!("{arg} is inside .git, which holds no files of the repository"),
        }
    }
}

/// The first line of `text`, trimmed; git's `fatal: ` dropped.
fn one_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("git failed");
    line.strip_prefix("fatal: ").unwrap_or(line).to_owned()
}

/// `text` on one line: its non-empty lines, trimmed, joined by `; `.
pub(super) fn flat(text: &str) -> String {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("; ")
}

/// `git rev-parse <args>` in `cwd`, as the user's own git would run it there; stdout, or the one-line error.
pub(super) fn rev_parse(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .current_dir(cwd)
        .env("LC_ALL", "C")
        .arg("rev-parse")
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(one_line(&String::from_utf8_lossy(&out.stderr)));
    }
    String::from_utf8(out.stdout).map_err(|_| "the repository's path is not UTF-8".to_owned())
}

impl Session {
    /// The repository at or above `cwd`; the error is the one line the command prints before it exits 3.
    pub(crate) fn open(cwd: &Path) -> Result<Session, String> {
        let repo = GitRepo::discover(cwd).map_err(|e| match e {
            GitError::NotARepo => "not a git repository: run this inside one".to_owned(),
            other => one_line(&other.to_string()),
        })?;
        let flags = rev_parse(cwd, &["--is-bare-repository", "--is-shallow-repository"])?;
        let mut flags = flags.lines().map(|l| l == "true");
        let (bare, shallow) = (flags.next().unwrap_or(false), flags.next().unwrap_or(false));
        if bare {
            return Ok(Session { repo, top: None, prefix: String::new(), shallow });
        }
        let out = rev_parse(cwd, &["--show-toplevel", "--show-prefix"])?;
        let mut lines = out.lines();
        let top = lines.next().filter(|t| !t.is_empty()).ok_or_else(|| "git printed no work tree".to_owned())?;
        let prefix = lines.next().unwrap_or_default().to_owned();
        Ok(Session { repo, top: Some(PathBuf::from(top)), prefix, shallow })
    }

    /// `arg` as a path from the work tree's top: `""` is the top itself.
    pub(crate) fn to_root(&self, arg: &str) -> Result<String, PathRefusal> {
        if !Path::new(arg).is_absolute() {
            return normalise(&format!("{}{arg}", self.prefix));
        }
        let top = self.top.as_deref().ok_or(PathRefusal::Outside)?;
        let spelled = lexical(Path::new(arg));
        if let Ok(rest) = spelled.strip_prefix(top) {
            return from_components(rest);
        }
        // The path may reach the work tree through a symlink (a temporary directory often does): resolve the
        // deepest part of it that exists, keep the rest as typed.
        let (mut base, mut tail) = (spelled.as_path(), Vec::new());
        loop {
            if let Ok(real) = std::fs::canonicalize(base) {
                let full = tail.iter().rev().fold(real, |p, s| p.join(s));
                return full.strip_prefix(top).map_err(|_| PathRefusal::Outside).and_then(from_components);
            }
            match (base.file_name(), base.parent()) {
                (Some(name), Some(parent)) => {
                    tail.push(name);
                    base = parent;
                }
                _ => return Err(PathRefusal::Outside),
            }
        }
    }
}

/// An absolute path with `.` and `..` resolved as text, as git resolves a pathspec.
fn lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn from_components(rest: &Path) -> Result<String, PathRefusal> {
    let parts: Option<Vec<&str>> = rest.components().map(|c| c.as_os_str().to_str()).collect();
    normalise(&parts.ok_or(PathRefusal::Outside)?.join("/"))
}

/// A path relative to the top, its `.` and empty segments dropped and each `..` taken back; one that climbs above
/// the top is outside, one with a `.git` segment is refused.
fn normalise(rel: &str) -> Result<String, PathRefusal> {
    let mut out: Vec<&str> = Vec::new();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop().ok_or(PathRefusal::Outside)?;
            }
            s => out.push(s),
        }
    }
    if out.iter().any(|s| s.eq_ignore_ascii_case(".git")) {
        return Err(PathRefusal::UnderGit);
    }
    Ok(out.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::testrepo::Repo;

    fn session(r: &Repo, sub: &str) -> Session {
        let cwd = r.dir().join(sub);
        std::fs::create_dir_all(&cwd).unwrap();
        Session::open(&cwd).unwrap()
    }

    #[test]
    fn flat_joins_the_lines_of_a_message() {
        assert_eq!(flat("fatal: a\n\n  b \r\n"), "fatal: a; b");
        assert_eq!(flat("one"), "one");
        assert_eq!(flat(""), "");
    }

    #[test]
    fn relative_to_cwd() {
        let r = Repo::new();
        assert_eq!(session(&r, "charts").to_root("api/values.yaml"), Ok("charts/api/values.yaml".to_owned()));
        assert_eq!(session(&r, "").to_root("charts/api/values.yaml"), Ok("charts/api/values.yaml".to_owned()));
        assert_eq!(session(&r, "charts/api").prefix, "charts/api/");
    }

    #[test]
    fn dot_segments() {
        let r = Repo::new();
        let s = session(&r, "charts/api");
        assert_eq!(s.to_root("./values.yaml"), Ok("charts/api/values.yaml".to_owned()));
        assert_eq!(s.to_root("../web/./x//y.yaml"), Ok("charts/web/x/y.yaml".to_owned()));
        assert_eq!(s.to_root("../../services/"), Ok("services".to_owned()));
        assert_eq!(s.to_root("../.."), Ok(String::new()));
        assert_eq!(s.to_root("."), Ok("charts/api".to_owned()));
    }

    #[test]
    fn absolute_inside() {
        let r = Repo::new();
        let s = session(&r, "charts");
        // The temporary directory may sit behind a symlink (macOS's /var); git reports the resolved top.
        let spelled = r.dir().join("services/billing/retry.rs");
        assert_eq!(s.to_root(spelled.to_str().unwrap()), Ok("services/billing/retry.rs".to_owned()));
        let resolved = std::fs::canonicalize(r.dir()).unwrap().join("charts/api");
        assert_eq!(s.to_root(resolved.to_str().unwrap()), Ok("charts/api".to_owned()));
        assert_eq!(s.to_root(std::fs::canonicalize(r.dir()).unwrap().to_str().unwrap()), Ok(String::new()));
    }

    #[test]
    fn absolute_outside_is_outside() {
        let r = Repo::new();
        let s = session(&r, "");
        let other = tempfile::tempdir().unwrap();
        assert_eq!(s.to_root(other.path().join("a.yaml").to_str().unwrap()), Err(PathRefusal::Outside));
        assert_eq!(s.to_root("/"), Err(PathRefusal::Outside));
        // A sibling whose name starts with the work tree's is not inside it.
        let sibling = format!("{}-other/a.yaml", std::fs::canonicalize(r.dir()).unwrap().display());
        assert_eq!(s.to_root(&sibling), Err(PathRefusal::Outside));
    }

    #[test]
    fn dotdot_escaping_is_outside() {
        let r = Repo::new();
        assert_eq!(session(&r, "charts").to_root("../../a.yaml"), Err(PathRefusal::Outside));
        assert_eq!(session(&r, "").to_root(".."), Err(PathRefusal::Outside));
        assert_eq!(session(&r, "a/b").to_root("../../../a/b/c"), Err(PathRefusal::Outside));
    }

    #[test]
    fn under_dot_git_is_refused() {
        let r = Repo::new();
        let s = session(&r, "charts");
        assert_eq!(session(&r, "").to_root(".git/config"), Err(PathRefusal::UnderGit));
        assert_eq!(s.to_root("../.git"), Err(PathRefusal::UnderGit));
        assert_eq!(s.to_root("sub/.GIT/hooks/pre-commit"), Err(PathRefusal::UnderGit));
        assert_eq!(s.to_root(r.dir().join(".git/HEAD").to_str().unwrap()), Err(PathRefusal::UnderGit));
        assert_eq!(s.to_root(".gitignore"), Ok("charts/.gitignore".to_owned()));
    }
}
