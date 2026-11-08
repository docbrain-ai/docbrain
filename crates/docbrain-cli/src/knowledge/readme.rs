// SPDX-License-Identifier: MIT
//! What `record` adds for people and agents on first use: the root `.knowledge/README.md`, and the lines that
//! point an agent's instructions file at the records, offered once and never staged.

use std::io::{BufRead, ErrorKind, Write};
use std::path::Path;

/// The root README, written once by the first `record` in a repository.
pub(crate) const README: &str = include_str!("readme.md");

/// Where the README goes, from the root.
pub(crate) const README_PATH: &str = ".knowledge/README.md";

/// The lines offered for the root `AGENTS.md` or `CLAUDE.md`.
pub(crate) const POINTER: &str = include_str!("pointer.md");

/// The root files an agent reads its instructions from, in the order they are named.
const INSTRUCTIONS: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];
const AGENTS: &str = INSTRUCTIONS[0];

/// Offers the pointer: on a terminal, one question, default no; otherwise the lines and where they go, printed.
/// Nothing when every root instructions file that exists already has them. Never staged: the files may hold
/// edits of the user's own.
pub(crate) fn offer_pointer(
    top: &Path,
    tty: bool,
    answer: &mut dyn BufRead,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let (lacking, any) = lacking(top);
    let targets: Vec<&str> = if any { lacking } else { vec![AGENTS] };
    if targets.is_empty() {
        return Ok(());
    }
    let place = if any { targets.join(" and ") } else { format!("a new {AGENTS}") };
    if !tty {
        let place = if any { place } else { format!("{place} at the repository root") };
        write!(out, "So that agents read these records, add these lines to {place}:\n{POINTER}")?;
        return Ok(out.flush()?);
    }
    write!(out, "{POINTER}Add these lines to {place}, so that agents read these records? [y/N] ")?;
    out.flush()?;
    let mut typed = String::new();
    if answer.read_line(&mut typed).is_err() || !matches!(typed.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        return Ok(());
    }
    for name in targets {
        match append(top, name) {
            Ok(true) => writeln!(out, "Added them to {name}; not staged.")?,
            Ok(false) => {}
            Err(e) => writeln!(out, "Could not add them to {name}: {e}")?,
        }
    }
    Ok(out.flush()?)
}

/// The instructions files at `top` that can be read and lack the heading, and whether any of them exists.
fn lacking(top: &Path) -> (Vec<&'static str>, bool) {
    let (mut lacking, mut any) = (Vec::new(), false);
    for name in INSTRUCTIONS {
        match std::fs::read(top.join(name)) {
            Ok(bytes) => {
                any = true;
                if !has_pointer(&bytes) {
                    lacking.push(name);
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(_) => any = true,
        }
    }
    (lacking, any)
}

fn has_pointer(bytes: &[u8]) -> bool {
    let heading = POINTER.lines().next().unwrap_or_default().as_bytes();
    bytes.split(|&b| b == b'\n').any(|l| l.trim_ascii_end() == heading)
}

/// Appends the pointer to `name`, created when missing, after reading it again: a file that has the heading by now
/// (the same file through a link, or an edit since the question) is left alone (`false`). The lines start a
/// paragraph of their own.
fn append(top: &Path, name: &str) -> std::io::Result<bool> {
    let path = top.join(name);
    let old = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    if has_pointer(&old) {
        return Ok(false);
    }
    let gap = if old.is_empty() || old.ends_with(b"\n\n") {
        ""
    } else if old.ends_with(b"\n") {
        "\n"
    } else {
        "\n\n"
    };
    let mut f = std::fs::OpenOptions::new().append(true).create(true).open(&path)?;
    f.write_all(format!("{gap}{POINTER}").as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::super::testrepo::Repo;
    use super::*;

    /// The run: what was printed.
    fn offer(r: &Repo, tty: bool, typed: &str) -> String {
        let mut out = Vec::new();
        offer_pointer(r.dir(), tty, &mut typed.as_bytes(), &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn read(r: &Repo, name: &str) -> Option<String> {
        std::fs::read_to_string(r.dir().join(name)).ok()
    }

    fn agents(text: &str) -> Repo {
        let r = Repo::new();
        r.write("AGENTS.md", text).commit("agents");
        r
    }

    #[test]
    fn pointer_yes_appends_and_never_stages() {
        let r = agents("# Agents\n");
        let out = offer(&r, true, "y\n");
        assert_eq!(read(&r, "AGENTS.md").unwrap(), format!("# Agents\n\n{POINTER}"));
        assert_eq!(read(&r, "CLAUDE.md"), None, "only the files that exist");
        assert!(out.starts_with(POINTER) && out.contains("Add these lines to AGENTS.md"), "{out}");
        assert!(out.contains("Added them to AGENTS.md; not staged.\n"), "{out}");
        assert_eq!(r.git(&["diff", "--cached", "--name-only"]), "");
        assert_eq!(r.git(&["status", "--porcelain"]), " M AGENTS.md\n");
    }

    #[test]
    fn pointer_no_changes_nothing() {
        for typed in ["\n", "n\n", "no\n", "", "maybe\n"] {
            let r = agents("# Agents\n");
            let out = offer(&r, true, typed);
            assert!(out.ends_with("? [y/N] "), "{typed:?}: {out}");
            assert_eq!(read(&r, "AGENTS.md").unwrap(), "# Agents\n", "{typed:?}");
            assert_eq!(read(&r, "CLAUDE.md"), None);
        }
    }

    #[test]
    fn yes_in_any_case_is_yes() {
        for typed in ["Y\n", "yes\n", " YES \n"] {
            let r = agents("# Agents\n");
            offer(&r, true, typed);
            assert_eq!(read(&r, "AGENTS.md").unwrap(), format!("# Agents\n\n{POINTER}"), "{typed:?}");
        }
    }

    #[test]
    fn with_neither_file_it_offers_a_new_agents_md() {
        let r = Repo::new();
        let out = offer(&r, true, "y\n");
        assert!(out.contains("Add these lines to a new AGENTS.md"), "{out}");
        assert_eq!(read(&r, "AGENTS.md").unwrap(), POINTER);
        assert_eq!(read(&r, "CLAUDE.md"), None);
    }

    #[test]
    fn it_names_only_the_files_that_lack_the_lines() {
        let r = Repo::new();
        r.write("AGENTS.md", format!("# Agents\n\n{POINTER}")).write("CLAUDE.md", "# Claude\n");
        let out = offer(&r, true, "y\n");
        assert!(out.contains("Add these lines to CLAUDE.md,"), "{out}");
        assert_eq!(read(&r, "AGENTS.md").unwrap().matches("## Repository knowledge").count(), 1);
        assert_eq!(read(&r, "CLAUDE.md").unwrap(), format!("# Claude\n\n{POINTER}"));

        r.write("AGENTS.md", "x\n## Repository knowledge  \ny\n");
        assert_eq!(offer(&r, true, "y\n"), "", "both have the heading: nothing asked");
    }

    #[test]
    fn pointer_yes_appends_once_through_a_symlink() {
        let r = Repo::new();
        r.write("CLAUDE.md", "# Claude\n");
        std::os::unix::fs::symlink("CLAUDE.md", r.dir().join("AGENTS.md")).unwrap();
        offer(&r, true, "y\n");
        assert_eq!(read(&r, "CLAUDE.md").unwrap(), format!("# Claude\n\n{POINTER}"));
    }

    #[test]
    fn pointer_after_a_file_without_a_final_newline() {
        let r = agents("# Agents");
        offer(&r, true, "y\n");
        assert_eq!(read(&r, "AGENTS.md").unwrap(), format!("# Agents\n\n{POINTER}"));
        let r = agents("# Agents\n\n");
        offer(&r, true, "y\n");
        assert_eq!(read(&r, "AGENTS.md").unwrap(), format!("# Agents\n\n{POINTER}"));
    }

    #[test]
    fn an_instructions_file_that_cannot_be_read_is_never_offered_a_new_one() {
        let r = Repo::new();
        std::fs::create_dir(r.dir().join("CLAUDE.md")).unwrap();
        assert_eq!(offer(&r, false, ""), "", "it exists: whether it has the lines is unknown");
        assert_eq!(offer(&r, true, "y\n"), "");
        assert!(read(&r, "AGENTS.md").is_none());
    }

    /// The file is read again just before the lines are added: one that cannot be read by then is left as it is
    /// and named, never appended to unread.
    #[test]
    fn a_file_that_cannot_be_read_by_the_answer_is_left_as_it_is() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: geteuid has no failure mode and reads only this process's effective user id.
        if unsafe { libc::geteuid() } == 0 {
            return; // permissions do not bind root
        }
        /// A reader that runs `.0` once, before its first read: the edit made while the question is open.
        struct First<F: FnMut()>(Option<F>, &'static [u8]);
        impl<F: FnMut()> std::io::Read for First<F> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if let Some(mut f) = self.0.take() {
                    f();
                }
                self.1.read(buf)
            }
        }
        let r = Repo::new();
        let path = r.dir().join(AGENTS);
        let edit = path.clone();
        let made_unreadable = move || {
            std::fs::write(&edit, "# Agents\n").unwrap();
            std::fs::set_permissions(&edit, std::fs::Permissions::from_mode(0o200)).unwrap();
        };
        let mut answer = std::io::BufReader::new(First(Some(made_unreadable), b"y\n"));
        let mut out = Vec::new();
        offer_pointer(r.dir(), true, &mut answer, &mut out).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("Could not add them to AGENTS.md: "), "{out}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Agents\n", "never appended unread");
    }

    #[test]
    fn a_pointer_that_cannot_be_written_keeps_exit_0() {
        let r = Repo::new();
        std::os::unix::fs::symlink("missing/AGENTS.md", r.dir().join("AGENTS.md")).unwrap();
        let out = offer(&r, true, "y\n");
        assert!(out.contains("Could not add them to AGENTS.md: "), "{out}");
    }

    #[test]
    fn no_terminal_prints_the_lines_with_where_they_go() {
        let r = agents("# Agents\n");
        let out = offer(&r, false, "y\n");
        assert_eq!(out, format!("So that agents read these records, add these lines to AGENTS.md:\n{POINTER}"));
        assert_eq!(read(&r, "AGENTS.md").unwrap(), "# Agents\n", "printed, never written");
        let r = Repo::new();
        assert!(offer(&r, false, "").contains("add these lines to a new AGENTS.md at the repository root:\n"));
    }

    /// `SEMANTICS.md` edge cases 82 and 86: the README says how to split a large records directory and how to keep
    /// the records out of what is built from their directory.
    #[test]
    fn the_readme_says_how_to_split_a_folder_and_keep_records_out_of_packages() {
        let flat = README.split_whitespace().collect::<Vec<_>>().join(" ");
        for advice in [
            "add the line `**/.knowledge` to `.dockerignore`, the line `.knowledge/` to `.helmignore` or `.npmignore`",
            "leave `.knowledge` out of `package.json`'s `files`",
            "When a `records/` directory passes 1,000 files, use one subdirectory per year; subdirectories carry no \
             meaning.",
        ] {
            assert!(flat.contains(advice), "{advice}");
        }
    }

    #[test]
    fn the_pointer_is_the_one_the_readme_and_the_design_give() {
        assert_eq!(POINTER.lines().count(), 5);
        assert!(README.starts_with("# Repository knowledge\n\n"));
    }
}
