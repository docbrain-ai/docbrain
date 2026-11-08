// SPDX-License-Identifier: MIT
//! A git repository built per test for the `.knowledge/` commands, and the binary run against it in a hermetic
//! environment: a cleared environment, a scratch `HOME`, no system or global git config, fixed names and dates.
//! Shared by the binary's integration tests and the knowledge module's unit tests.
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What one run of the binary gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub struct Repo {
    scratch: tempfile::TempDir,
    dir: PathBuf,
    home: tempfile::TempDir,
}

/// The variables every git and binary run gets, after the environment is cleared.
pub fn hermetic(c: &mut Command, home: &Path) {
    c.env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Fixture Author")
        .env("GIT_AUTHOR_EMAIL", "author@example.invalid")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_NAME", "Fixture Committer")
        .env("GIT_COMMITTER_EMAIL", "committer@example.invalid")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z");
}

/// The binary under test: set by cargo for integration tests; the unit tests that share this file never run it.
pub fn binary() -> PathBuf {
    match option_env!("CARGO_BIN_EXE_docbrain-cli") {
        Some(p) => PathBuf::from(p),
        None => panic!("the binary is known only to the integration tests"),
    }
}

impl Repo {
    /// An empty repository on branch `main`.
    pub fn new() -> Repo {
        let scratch = tempfile::tempdir().unwrap();
        let dir = scratch.path().to_owned();
        Repo::init(scratch, dir)
    }

    /// An empty repository on branch `main` in a directory named `name`, for a path that holds given characters.
    pub fn at(name: &str) -> Repo {
        let scratch = tempfile::tempdir().unwrap();
        let dir = scratch.path().join(name);
        std::fs::create_dir(&dir).unwrap();
        Repo::init(scratch, dir)
    }

    fn init(scratch: tempfile::TempDir, dir: PathBuf) -> Repo {
        let r = Repo { scratch, dir, home: tempfile::tempdir().unwrap() };
        r.git(&["init", "-q", "-b", "main"]);
        r
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn home(&self) -> &Path {
        self.home.path()
    }

    /// Writes `bytes` at `path` (relative to the work tree), creating its directories.
    pub fn write(&self, path: &str, bytes: impl AsRef<[u8]>) -> &Repo {
        let p = self.dir().join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
        self
    }

    /// Runs git in the work tree; panics on failure; stdout.
    pub fn git(&self, args: &[&str]) -> String {
        self.git_in(self.dir(), args)
    }

    pub fn git_in(&self, cwd: &Path, args: &[&str]) -> String {
        let mut c = Command::new("git");
        hermetic(&mut c, self.home());
        let out = c.current_dir(cwd).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }

    /// Stages everything and commits it; the new commit's id.
    pub fn commit(&self, msg: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--allow-empty", "-m", msg]);
        self.git(&["rev-parse", "HEAD"]).trim().to_owned()
    }

    /// The binary in the work tree's root.
    pub fn run(&self, args: &[&str]) -> Out {
        self.run_with(self.dir(), &[], None, args)
    }

    /// The binary in `cwd`, with `env` added and `stdin` written to it (closed, empty, when `None`).
    pub fn run_with(&self, cwd: &Path, env: &[(&str, &str)], stdin: Option<&[u8]>, args: &[&str]) -> Out {
        let mut c = Command::new(binary());
        hermetic(&mut c, self.home());
        c.current_dir(cwd).args(args).envs(env.iter().copied()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = c.spawn().unwrap();
        let mut input = child.stdin.take().unwrap();
        if let Some(bytes) = stdin {
            input.write_all(bytes).unwrap();
        }
        drop(input);
        let out = child.wait_with_output().unwrap();
        Out {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8(out.stdout).unwrap(),
            stderr: String::from_utf8(out.stderr).unwrap(),
        }
    }
}
